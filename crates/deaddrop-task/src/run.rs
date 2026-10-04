//! Running a validated plan through Agent Wire.
//!
//! Every step is an Agent Wire quest in one history named after the task:
//! the orchestrator ASSIGNs it when its dependencies are complete, the
//! worker's room report is recorded as that worker's ACCEPT + RESULT (or
//! NEED_HUMAN when it refused or failed), and the orchestrator EVALUATEs it
//! against the quest's acceptance gates. Agent Wire decides COMPLETE; a
//! worker has no way to say it. The task completes once, when Agent Wire
//! holds every quest COMPLETE.
//!
//! Independent steps are assigned together, so their workers run at the
//! same time; a step is assigned only after every step it depends on is
//! complete. Nothing here sends: the caller delivers each [`Dispatch`] as
//! an ordinary room request and feeds reports back in.

use agent_wire_coordinate::{
    ArtifactRef, CoordinationError, GateCheck, GateId, HistoryId, Orchestrator, QuestId,
    QuestResult, QuestSpec, QuestState, Verdict, WorkerId, WorkerReport,
};
use deaddrop_room::{RoomMessage, Status, short};

use crate::plan::{Plan, Step};
use crate::registry::{GITHUB_INSPECT, SYNTHESIZE, WEB_SEARCH};

/// How long a dispatched step may go without a report.
pub const STEP_TIMEOUT_MS: u64 = 150_000;
/// Longest part of one report handed to synthesis.
pub const MAX_REPORT_CHARS: usize = 1500;
/// Every quest's gate: the worker reported, and said ok.
pub const REPORT_OK: &str = "report_ok";

/// The gates Agent Wire checks a step's result against.
pub fn gates(capability: &str) -> Vec<&'static str> {
    match capability {
        WEB_SEARCH => vec![REPORT_OK, "sources_cited"],
        GITHUB_INSPECT => vec![REPORT_OK, "inspector_source"],
        _ => vec![REPORT_OK],
    }
}

/// The deterministic gate observations for a report, each with the report
/// id as its evidence. Facts about the text, never a judgment of it.
fn checks(capability: &str, report_id: &str, status: Status, text: &str) -> Vec<GateCheck> {
    let check = |gate: &str, passed: bool| GateCheck {
        gate: GateId::new(gate),
        passed,
        evidence: Some(ArtifactRef::new(report_id)),
    };
    gates(capability)
        .into_iter()
        .map(|gate| match gate {
            REPORT_OK => check(gate, status == Status::Ok),
            "sources_cited" => check(
                gate,
                text.lines().any(|l| {
                    let l = l.trim();
                    l.starts_with("https://") || l.starts_with("http://")
                }),
            ),
            "inspector_source" => check(gate, text.contains("source: github.inspect")),
            other => check(other, false),
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    /// Not yet assigned: waiting for its dependencies.
    Waiting,
    /// Assigned in Agent Wire and sent; no report yet.
    Running {
        request: String,
        since_ms: u64,
    },
    /// Agent Wire holds the quest COMPLETE.
    Complete,
    Failed(String),
    /// A dependency failed, so this step is never assigned.
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: String,
    pub from: String,
    pub status: Status,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Failed(String),
}

/// One real orchestration event, for `0xd34ddr0p::wire`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trace {
    pub from: String,
    pub to: Option<String>,
    pub fields: Vec<(&'static str, String)>,
}

/// A step to deliver: a room request from this node to `worker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispatch {
    pub step: String,
    pub worker: String,
    /// The room message id to use, so reports can be matched to it.
    pub request: String,
    pub text: String,
}

#[derive(Debug)]
pub struct TaskRun {
    pub id: String,
    pub plan: Plan,
    pub states: Vec<StepState>,
    pub reports: Vec<Option<Report>>,
    pub trace: Vec<Trace>,
    pub outcome: Option<Outcome>,
    wire: Orchestrator,
}

/// A copy rebuilt from the recorded history: Agent Wire re-validates every
/// event, so a clone can never hold a state the original could not.
impl Clone for TaskRun {
    fn clone(&self) -> Self {
        let workers = self.plan.workers().into_iter().map(WorkerId::new);
        Self {
            id: self.id.clone(),
            plan: self.plan.clone(),
            states: self.states.clone(),
            reports: self.reports.clone(),
            trace: self.trace.clone(),
            outcome: self.outcome.clone(),
            wire: Orchestrator::from_events(
                self.wire.history().clone(),
                self.wire.events().to_vec(),
                workers,
            )
            .expect("a recorded history replays"),
        }
    }
}

const ORCHESTRATOR: &str = "orchestrator";

fn trace(from: &str, to: Option<&str>, fields: Vec<(&'static str, String)>) -> Trace {
    Trace {
        from: from.to_owned(),
        to: to.map(str::to_owned),
        fields,
    }
}

impl TaskRun {
    /// Accept a validated plan. Nothing is assigned until [`TaskRun::ready`].
    pub fn new(id: &str, plan: Plan) -> Self {
        let workers = plan
            .workers()
            .into_iter()
            .map(WorkerId::new)
            .collect::<Vec<_>>();
        let n = plan.steps.len();
        let mut run = Self {
            id: id.to_owned(),
            states: vec![StepState::Waiting; n],
            reports: vec![None; n],
            trace: Vec::new(),
            outcome: None,
            wire: Orchestrator::new(HistoryId::new(id), workers),
            plan,
        };
        run.trace.push(trace(
            ORCHESTRATOR,
            None,
            vec![("plan", format!("accepted · {n} steps"))],
        ));
        run
    }

    /// The Agent Wire quest id of a step.
    pub fn quest(&self, step: &str) -> QuestId {
        QuestId::new(format!("{}.{step}", self.id))
    }

    pub fn wire(&self) -> &Orchestrator {
        &self.wire
    }

    /// Assign every step whose dependencies are all complete, and block
    /// every step one of whose dependencies failed. `new_id` names each
    /// room request. Independent steps come back together.
    pub fn ready(&mut self, now_ms: u64, mut new_id: impl FnMut() -> String) -> Vec<Dispatch> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..self.plan.steps.len() {
            if self.states[i] != StepState::Waiting {
                continue;
            }
            let step = self.plan.steps[i].clone();
            let deps: Vec<usize> = step
                .depends_on
                .iter()
                .filter_map(|d| self.plan.steps.iter().position(|s| &s.id == d))
                .collect();
            if let Some(&bad) = deps
                .iter()
                .find(|&&d| matches!(self.states[d], StepState::Failed(_) | StepState::Blocked(_)))
            {
                let why = format!("{} did not complete", self.plan.steps[bad].id);
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("blocked", step.id.clone()), ("reason", why.clone())],
                ));
                self.states[i] = StepState::Blocked(why);
                continue;
            }
            if !deps.iter().all(|&d| self.states[d] == StepState::Complete) {
                continue;
            }
            let text = match step.capability {
                SYNTHESIZE => self.synthesis_input(&step, &deps),
                _ => step.request.clone(),
            };
            let spec = QuestSpec {
                id: self.quest(&step.id),
                parent_quest_id: None,
                assignee: WorkerId::new(&step.worker),
                objective: step.objective.clone(),
                constraints: vec!["read-only".into(), "no side effects".into()],
                acceptance_gates: gates(step.capability)
                    .into_iter()
                    .map(GateId::new)
                    .collect(),
            };
            let seq = match self.wire.assign(spec) {
                Ok(seq) => seq,
                Err(e) => {
                    self.fail(i, format!("agent wire refused assign: {e}"));
                    continue;
                }
            };
            let mut fields = vec![
                ("assign", step.capability.to_owned()),
                ("step", step.id.clone()),
            ];
            if !step.depends_on.is_empty() {
                fields.push(("depends_on", step.depends_on.join(",")));
            }
            fields.push(("event", format!("ASSIGN #{seq}")));
            self.trace
                .push(trace(ORCHESTRATOR, Some(short(&step.worker)), fields));
            let request = new_id();
            self.states[i] = StepState::Running {
                request: request.clone(),
                since_ms: now_ms,
            };
            out.push(Dispatch {
                step: step.id.clone(),
                worker: step.worker.clone(),
                request,
                text: format!("task:: {} · {}\n{text}", self.id, step.id),
            });
        }
        self.settle();
        out
    }

    /// What synthesis is given: only the reports it depends on, and the
    /// objective. No other room traffic, no completion authority.
    fn synthesis_input(&self, step: &Step, deps: &[usize]) -> String {
        let mut text = String::new();
        for &d in deps {
            let s = &self.plan.steps[d];
            let r = self.reports[d]
                .as_ref()
                .expect("a complete step has its report");
            text.push_str(&format!(
                "REPORT {} ({} · {})\n{}\n\n",
                s.id,
                s.capability,
                short(&s.worker),
                deaddrop_klodik::model::cut(r.text.trim(), MAX_REPORT_CHARS)
            ));
        }
        text.push_str(&format!("synthesize:: {}", step.objective));
        text
    }

    /// Take in a room report. Only a report replying to a running step's
    /// request counts, and Agent Wire records it only if it comes from that
    /// step's assignee. Returns whether anything changed.
    pub fn observe(&mut self, from: &str, report: &RoomMessage) -> bool {
        if self.outcome.is_some() || report.kind != deaddrop_room::Kind::Report {
            return false;
        }
        let Some(reply_to) = report.reply_to.as_deref() else {
            return false;
        };
        let Some(i) = self
            .states
            .iter()
            .position(|s| matches!(s, StepState::Running { request, .. } if request == reply_to))
        else {
            return false;
        };
        let step = self.plan.steps[i].clone();
        let quest = self.quest(&step.id);
        let worker = WorkerId::new(from);
        let status = report.status.unwrap_or(Status::Ok);
        let recorded = match status {
            Status::Ok => self
                .wire
                .report(&worker, &quest, WorkerReport::Accept)
                .and_then(|_| {
                    self.wire.report(
                        &worker,
                        &quest,
                        WorkerReport::Result(QuestResult {
                            summary: report
                                .text
                                .lines()
                                .next()
                                .unwrap_or("")
                                .chars()
                                .take(120)
                                .collect(),
                            artifacts: vec![ArtifactRef::new(&report.id)],
                            gate_checks: checks(step.capability, &report.id, status, &report.text),
                        }),
                    )
                }),
            Status::Refused | Status::Failed => self.wire.report(
                &worker,
                &quest,
                WorkerReport::NeedHuman {
                    reason: format!(
                        "{}: {}",
                        status.as_str(),
                        report.text.lines().next().unwrap_or("")
                    ),
                },
            ),
        };
        if let Err(e) = recorded {
            // Not the assignee, or not a legal report: Agent Wire records
            // nothing and the step keeps waiting for its real worker.
            let reason = match e {
                CoordinationError::NotAuthorized { .. } => "not the assignee".to_owned(),
                other => other.to_string(),
            };
            self.trace.push(trace(
                short(from),
                Some(ORCHESTRATOR),
                vec![("rejected", step.id.clone()), ("reason", reason)],
            ));
            return true;
        }
        self.reports[i] = Some(Report {
            id: report.id.clone(),
            from: from.to_owned(),
            status,
            text: report.text.clone(),
        });
        self.trace.push(trace(
            short(from),
            Some(ORCHESTRATOR),
            vec![
                ("report", step.id.clone()),
                (
                    "status",
                    match status {
                        Status::Ok => "result".to_owned(),
                        other => format!("need_human · {}", other.as_str()),
                    },
                ),
            ],
        ));
        if status != Status::Ok {
            self.fail(i, format!("{} {}", short(from), status.as_str()));
            return true;
        }
        match self.wire.evaluate(&quest) {
            Ok(Verdict::Complete) => {
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("evaluate", format!("{} · gates met · COMPLETE", step.id))],
                ));
                self.states[i] = StepState::Complete;
            }
            Ok(Verdict::RevisionRequired { unmet_gates }) => {
                let unmet: Vec<&str> = unmet_gates.iter().map(|g| g.as_str()).collect();
                // V0 never revises or replans: an unmet gate fails the step.
                self.fail(i, format!("gates unmet: {}", unmet.join(",")));
            }
            Err(e) => self.fail(i, format!("agent wire: {e}")),
        }
        self.settle();
        true
    }

    /// Fail running steps that have waited past [`STEP_TIMEOUT_MS`]. Agent
    /// Wire has no timeout event, so its quest stays open; the task does not.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        if self.outcome.is_some() {
            return false;
        }
        let late: Vec<usize> = (0..self.states.len())
            .filter(|&i| {
                matches!(self.states[i], StepState::Running { since_ms, .. }
                    if now_ms.saturating_sub(since_ms) > STEP_TIMEOUT_MS)
            })
            .collect();
        for &i in &late {
            self.fail(
                i,
                format!("timeout: no report in {}s", STEP_TIMEOUT_MS / 1000),
            );
        }
        if !late.is_empty() {
            self.settle();
        }
        !late.is_empty()
    }

    fn fail(&mut self, i: usize, why: String) {
        self.trace.push(trace(
            ORCHESTRATOR,
            None,
            vec![
                ("failed", self.plan.steps[i].id.clone()),
                ("reason", why.clone()),
            ],
        ));
        self.states[i] = StepState::Failed(why);
    }

    /// Decide the task, once, when nothing can move any more. Complete only
    /// if Agent Wire itself holds every step's quest COMPLETE.
    fn settle(&mut self) {
        if self.outcome.is_some() {
            return;
        }
        let moving = self.states.iter().enumerate().any(|(i, s)| match s {
            StepState::Running { .. } => true,
            StepState::Waiting => !self.plan.steps[i].depends_on.iter().any(|d| {
                let j = self
                    .plan
                    .steps
                    .iter()
                    .position(|t| &t.id == d)
                    .expect("validated");
                matches!(self.states[j], StepState::Failed(_) | StepState::Blocked(_))
            }),
            _ => false,
        });
        if moving {
            return;
        }
        let quests = self.wire.quests();
        let complete = self.plan.steps.iter().all(|s| {
            quests
                .get(&self.quest(&s.id))
                .is_some_and(|q| q.state == QuestState::Complete)
        });
        if complete {
            self.trace
                .push(trace(ORCHESTRATOR, None, vec![("evaluate", "pass".into())]));
            self.trace.push(trace(
                ORCHESTRATOR,
                None,
                vec![("complete", self.id.clone())],
            ));
            self.outcome = Some(Outcome::Complete);
        } else {
            let failed: Vec<String> = self
                .plan
                .steps
                .iter()
                .zip(&self.states)
                .filter_map(|(s, st)| match st {
                    StepState::Failed(why) => Some(format!("{} failed ({why})", s.id)),
                    StepState::Blocked(_) => Some(format!("{} blocked", s.id)),
                    StepState::Waiting => Some(format!("{} blocked", s.id)),
                    _ => None,
                })
                .collect();
            let why = failed.join("; ");
            self.trace.push(trace(
                ORCHESTRATOR,
                None,
                vec![("evaluate", format!("fail · {why}"))],
            ));
            self.outcome = Some(Outcome::Failed(why));
        }
    }

    /// The final report, if the task completed: the last step's report.
    pub fn result(&self) -> Option<&Report> {
        match self.outcome {
            Some(Outcome::Complete) => self.reports.last().and_then(Option::as_ref),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::tests::{TASK, ctx, golden};
    use crate::plan::validate;
    use agent_wire_coordinate::EventKind;
    use deaddrop_room::Kind;

    const RESEARCH: &str = "research:agent:deaddrop";
    const GITHUB: &str = "github:agent:deaddrop";
    const KLODIK: &str = "klodik:agent:deaddrop";

    fn start() -> (TaskRun, Vec<Dispatch>) {
        let mut run = TaskRun::new("T-1042", validate(TASK, &golden(), &ctx()).unwrap());
        let mut n = 0;
        let sent = run.ready(0, || {
            n += 1;
            format!("rq{n}")
        });
        (run, sent)
    }

    fn report(to: &Dispatch, status: Status, text: &str) -> RoomMessage {
        RoomMessage {
            room: "d34ddr0p".into(),
            id: format!("re-{}", to.request),
            kind: Kind::Report,
            hop: 0,
            mentions: vec![],
            reply_to: Some(to.request.clone()),
            status: Some(status),
            text: text.into(),
        }
    }

    const FOUND: &str = "Rust 1.99.0, October 1, 2026.\n\nSources:\n1. Announcing Rust 1.99.0\n   https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/";
    const YES: &str = "yes, it modifies crates/deaddrop-tui/src/ui.rs\no/r@sha\nsource: github.inspect · commit_modifies_path (github-inspector, read-only)";

    fn next(run: &mut TaskRun) -> Vec<Dispatch> {
        run.ready(1, || "rq-next".into())
    }

    #[test]
    fn independent_steps_are_assigned_together_and_synthesis_waits_for_both() {
        let (mut run, sent) = start();
        let workers: Vec<&str> = sent.iter().map(|d| d.worker.as_str()).collect();
        assert_eq!(
            workers,
            [RESEARCH, GITHUB],
            "both at once, before any report"
        );
        assert!(
            sent[1]
                .text
                .starts_with("task:: T-1042 · inspect_commit\nrequest:: github.inspect")
        );
        // GitHub finishes first; Research is still running.
        assert!(run.observe(GITHUB, &report(&sent[1], Status::Ok, YES)));
        assert!(next(&mut run).is_empty(), "synthesis waits for research");
        assert_eq!(run.outcome, None);
        assert!(run.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND)));
        let synth = next(&mut run);
        assert_eq!(synth.len(), 1);
        assert_eq!(synth[0].worker, KLODIK);
        // Only the reports it depends on, and the objective.
        let text = &synth[0].text;
        assert!(text.contains("REPORT research_release (web.search · research)\nRust 1.99.0"));
        assert!(text.contains("REPORT inspect_commit (github.inspect · github)\nyes, it modifies"));
        assert!(text.ends_with("synthesize:: Summarize both findings."));
        assert_eq!(run.outcome, None, "a worker report is not completion");
        assert!(run.observe(
            KLODIK,
            &report(&synth[0], Status::Ok, "Both: 1.99.0, and yes.")
        ));
        assert_eq!(run.outcome, Some(Outcome::Complete));
        assert_eq!(run.result().unwrap().text, "Both: 1.99.0, and yes.");
    }

    #[test]
    fn agent_wire_holds_the_lifecycle_and_completes_exactly_once() {
        let (mut run, sent) = start();
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES));
        run.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND));
        let synth = next(&mut run);
        run.observe(KLODIK, &report(&synth[0], Status::Ok, "done"));
        // Replays change nothing.
        assert!(!run.observe(KLODIK, &report(&synth[0], Status::Ok, "done")));
        assert!(next(&mut run).is_empty());
        let names: Vec<String> = run
            .wire()
            .events()
            .iter()
            .map(|e| {
                format!(
                    "{}:{}",
                    e.quest_id.as_str().rsplit('.').next().unwrap(),
                    e.kind.name()
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                "research_release:ASSIGN",
                "inspect_commit:ASSIGN",
                "inspect_commit:ACCEPT",
                "inspect_commit:RESULT",
                "inspect_commit:COMPLETE",
                "research_release:ACCEPT",
                "research_release:RESULT",
                "research_release:COMPLETE",
                "synthesize:ASSIGN",
                "synthesize:ACCEPT",
                "synthesize:RESULT",
                "synthesize:COMPLETE",
            ]
        );
        // Every COMPLETE is the orchestrator's.
        for e in run.wire().events() {
            if matches!(e.kind, EventKind::Complete { .. }) {
                assert_eq!(e.actor, agent_wire_coordinate::Actor::Orchestrator);
            }
        }
        let completes = run
            .trace
            .iter()
            .filter(|t| t.fields[0].0 == "complete")
            .count();
        assert_eq!(completes, 1, "the task completes exactly once");
    }

    #[test]
    fn a_worker_cannot_report_for_another_or_complete_the_task() {
        let (mut run, sent) = start();
        // Klodik answers GitHub's request: Agent Wire refuses, nothing recorded.
        assert!(run.observe(KLODIK, &report(&sent[1], Status::Ok, YES)));
        assert!(matches!(run.states[1], StepState::Running { .. }));
        assert_eq!(
            run.trace.last().unwrap().fields[1],
            ("reason", "not the assignee".into())
        );
        // A worker's message claiming completion is just a message.
        let mut claim = report(&sent[0], Status::Ok, "task:: complete");
        claim.kind = Kind::Message;
        assert!(!run.observe(RESEARCH, &claim));
        assert_eq!(run.outcome, None);
    }

    #[test]
    fn a_failure_keeps_other_reports_and_blocks_what_depends_on_it() {
        let (mut run, sent) = start();
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES));
        run.observe(
            RESEARCH,
            &report(&sent[0], Status::Failed, "research failed: timeout"),
        );
        assert!(next(&mut run).is_empty(), "synthesis is never assigned");
        assert_eq!(run.states[1], StepState::Complete, "github's result stands");
        assert_eq!(run.reports[1].as_ref().unwrap().text, YES);
        assert!(matches!(run.states[2], StepState::Blocked(_)));
        let Some(Outcome::Failed(why)) = &run.outcome else {
            panic!("{:?}", run.outcome)
        };
        assert_eq!(
            why,
            "research_release failed (research failed); synthesize blocked"
        );
        let need_human = run
            .wire()
            .events()
            .iter()
            .any(|e| e.kind.name() == "NEED_HUMAN");
        assert!(
            need_human,
            "recorded in Agent Wire as the worker's own report"
        );
    }

    #[test]
    fn unmet_gates_fail_the_step_without_revision() {
        let (mut run, sent) = start();
        run.observe(
            RESEARCH,
            &report(&sent[0], Status::Ok, "Rust 1.99.0 (no sources)"),
        );
        assert_eq!(
            run.states[0],
            StepState::Failed("gates unmet: sources_cited".into())
        );
        assert!(
            run.wire()
                .events()
                .iter()
                .any(|e| e.kind.name() == "REVISE")
        );
    }

    #[test]
    fn a_silent_worker_times_out() {
        let (mut run, sent) = start();
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES));
        assert!(!run.expire(STEP_TIMEOUT_MS));
        assert!(run.expire(STEP_TIMEOUT_MS + 1));
        assert!(matches!(&run.outcome, Some(Outcome::Failed(w)) if w.contains("timeout")));
    }
}
