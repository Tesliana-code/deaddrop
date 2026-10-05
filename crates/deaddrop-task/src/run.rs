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
//!
//! A run is the replay of its [`journal`](crate::journal) records and
//! nothing else: every transition is a [`Record`], applied by the same
//! code live and on replay, and Agent Wire's events are records too, so
//! [`TaskRun::replay`] rebuilds exactly the state, the trace and Agent
//! Wire's history — re-validated — that the live run had.

use std::collections::HashSet;

use agent_wire_coordinate::{
    Actor, ArtifactRef, CoordinationError, EventKind, GateCheck, GateId, HistoryId, Orchestrator,
    QuestId, QuestResult, QuestSpec, QuestState, Verdict, WorkerId, WorkerReport,
};
use deaddrop_room::{Kind, RoomMessage, Status, short};

use crate::journal::{JournalError, PlannedStep, Record};
use crate::plan::{Plan, Step};
use crate::registry::{
    Capability, Context, GITHUB_INSPECT, Mode, REGISTRY, Refusal, SYNTHESIZE, WEB_SEARCH,
};

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

pub fn parse_status(s: &str) -> Option<Status> {
    match s {
        "ok" => Some(Status::Ok),
        "refused" => Some(Status::Refused),
        "failed" => Some(Status::Failed),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState {
    /// Not yet assigned: waiting for its dependencies.
    Waiting,
    /// Assigned in Agent Wire and sent; no report yet. `since_ms` is wall
    /// clock, so a timeout survives a restart.
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
    /// The report's text, cut to [`MAX_REPORT_CHARS`]: what synthesis may
    /// be handed. The whole text stays in signed history under `id`.
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

/// What the task was, as accepted from the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub room: String,
    /// The human's words, as written.
    pub objective: String,
    pub human_message: String,
    pub accepted_message: String,
    pub at_ms: u64,
}

/// A room message the task's journal says was meant to be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub kind: Kind,
    pub mentions: Vec<String>,
    pub text: String,
    /// The wire's (lifecycle line or step request), not the human's.
    pub wire: bool,
}

#[derive(Debug)]
pub struct TaskRun {
    pub id: String,
    pub accepted: Accepted,
    pub plan: Plan,
    pub states: Vec<StepState>,
    pub reports: Vec<Option<Report>>,
    pub trace: Vec<Trace>,
    pub outcome: Option<Outcome>,
    wire: Orchestrator,
    /// Agent Wire events already journaled.
    wire_logged: usize,
    journal: Vec<Record>,
    /// Room reports already taken in (recorded or refused).
    observed: HashSet<String>,
    /// Each step's room request: (id, text).
    requests: Vec<Option<(String, String)>>,
    announced: Option<String>,
    terminal_at_ms: Option<u64>,
}

/// A copy is the replay of the journal: it can never hold a state the
/// original could not.
impl Clone for TaskRun {
    fn clone(&self) -> Self {
        Self::replay(&self.id, &self.journal).expect("a live journal replays")
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

impl From<&Step> for PlannedStep {
    fn from(s: &Step) -> Self {
        Self {
            id: s.id.clone(),
            capability: s.capability.to_owned(),
            worker: s.worker.clone(),
            mode: s.mode.as_str().to_owned(),
            objective: s.objective.clone(),
            depends_on: s.depends_on.clone(),
            request: s.request.clone(),
        }
    }
}

/// A recorded plan back into a plan. The journal grants nothing: each
/// capability must be a registered one, served by its registered peer.
fn plan_of(steps: &[PlannedStep]) -> Result<Plan, String> {
    let mut out: Vec<Step> = Vec::new();
    for p in steps {
        let c = crate::registry::capability(&p.capability)
            .ok_or_else(|| format!("unknown capability {:?}", p.capability))?;
        if c.peer != p.worker {
            return Err(format!("{} is not served by {}", c.id, p.worker));
        }
        let mode = match p.mode.as_str() {
            "read-only" => Mode::ReadOnly,
            "report-only" => Mode::ReportOnly,
            other => return Err(format!("unknown mode {other:?}")),
        };
        if out.iter().any(|s| s.id == p.id) {
            return Err(format!("duplicate step id {:?}", p.id));
        }
        out.push(Step {
            id: p.id.clone(),
            capability: c.id,
            worker: p.worker.clone(),
            mode,
            objective: p.objective.clone(),
            depends_on: p.depends_on.clone(),
            request: p.request.clone(),
        });
    }
    if out.is_empty() {
        return Err("empty plan".into());
    }
    for s in &out {
        if let Some(d) = s
            .depends_on
            .iter()
            .find(|d| !out.iter().any(|t| &t.id == *d))
        {
            return Err(format!("step {} depends on missing step {d:?}", s.id));
        }
    }
    crate::plan::acyclic(&out)?;
    Ok(Plan { steps: out })
}

/// Whether `step` may be dispatched now: the current registry must still
/// route its capability to its worker with the authority it was accepted
/// with, and the worker must be trusted, in the room and allowed by policy
/// now. Capability alone never wins.
pub fn authorize(registry: &[Capability], step: &Step, ctx: &Context) -> Result<(), String> {
    let (cap, who) = (step.capability, short(&step.worker));
    let c = crate::registry::resolve_in(registry, cap, ctx).map_err(|e| match e {
        Refusal::UnknownCapability(_) => format!("{cap} is no longer a registered capability"),
        Refusal::Untrusted { .. } => format!("{who} is no longer a trusted peer"),
        Refusal::NotInRoom { .. } => format!("{who} is no longer in the room"),
        Refusal::PolicyDenied { .. } => {
            format!("current policy no longer permits {cap} via {who}")
        }
        Refusal::Authority { .. } => format!("{cap} needs authority V0 does not grant"),
    })?;
    if c.peer != step.worker {
        return Err(format!("{cap} is no longer served by {who}"));
    }
    if c.mode != step.mode {
        return Err(format!(
            "{cap} authority changed ({} → {})",
            step.mode.as_str(),
            c.mode.as_str()
        ));
    }
    Ok(())
}

impl TaskRun {
    /// Accept a validated plan. Nothing is assigned until [`TaskRun::ready`].
    pub fn accept(id: &str, accepted: Accepted, plan: Plan) -> Self {
        let records = vec![
            Record::TaskAccepted {
                room: accepted.room,
                objective: accepted.objective,
                human_message: accepted.human_message,
                accepted_message: accepted.accepted_message,
                at_ms: accepted.at_ms,
            },
            Record::PlanValidated {
                steps: plan.steps.iter().map(PlannedStep::from).collect(),
            },
        ];
        Self::replay(id, &records).expect("a validated plan replays")
    }

    /// Rebuild a run from its journal records, refusing any record that is
    /// not a legal transition from the state before it.
    pub fn replay(id: &str, records: &[Record]) -> Result<Self, JournalError> {
        let err = |i: usize, reason: String| JournalError::Replay {
            record: i + 1,
            reason,
        };
        let Some(Record::TaskAccepted {
            room,
            objective,
            human_message,
            accepted_message,
            at_ms,
        }) = records.first()
        else {
            return Err(err(0, "the first record is not task_accepted".into()));
        };
        let Some(Record::PlanValidated { steps }) = records.get(1) else {
            return Err(err(1, "the second record is not plan_validated".into()));
        };
        let plan = plan_of(steps).map_err(|e| err(1, e))?;
        let workers: Vec<WorkerId> = plan.workers().into_iter().map(WorkerId::new).collect();
        let n = plan.steps.len();
        let mut run = Self {
            id: id.to_owned(),
            accepted: Accepted {
                room: room.clone(),
                objective: objective.clone(),
                human_message: human_message.clone(),
                accepted_message: accepted_message.clone(),
                at_ms: *at_ms,
            },
            states: vec![StepState::Waiting; n],
            reports: vec![None; n],
            trace: vec![trace(
                ORCHESTRATOR,
                None,
                vec![("plan", format!("accepted · {n} steps"))],
            )],
            outcome: None,
            wire: Orchestrator::new(HistoryId::new(id), workers),
            wire_logged: 0,
            journal: records[..2].to_vec(),
            observed: HashSet::new(),
            requests: vec![None; n],
            announced: None,
            terminal_at_ms: None,
            plan,
        };
        for (i, record) in records.iter().enumerate().skip(2) {
            run.apply(record).map_err(|e| err(i, e))?;
            run.journal.push(record.clone());
        }
        Ok(run)
    }

    /// Every record so far, in order: what the journal must hold.
    pub fn journal(&self) -> &[Record] {
        &self.journal
    }

    /// The Agent Wire quest id of a step.
    pub fn quest(&self, step: &str) -> QuestId {
        QuestId::new(format!("{}.{step}", self.id))
    }

    pub fn wire(&self) -> &Orchestrator {
        &self.wire
    }

    /// The room message announcing the outcome, once sent.
    pub fn announced(&self) -> Option<&str> {
        self.announced.as_deref()
    }

    /// When the task reached its outcome (wall clock ms).
    pub fn terminal_at_ms(&self) -> Option<u64> {
        self.terminal_at_ms
    }

    /// Each step's room request id, if it was assigned.
    pub fn request(&self, i: usize) -> Option<&str> {
        self.requests[i].as_ref().map(|(id, _)| id.as_str())
    }

    fn index(&self, step: &str) -> Result<usize, String> {
        self.plan
            .steps
            .iter()
            .position(|s| s.id == step)
            .ok_or_else(|| format!("no step {step:?}"))
    }

    fn deps(&self, i: usize) -> Vec<usize> {
        self.plan.steps[i]
            .depends_on
            .iter()
            .filter_map(|d| self.plan.steps.iter().position(|s| &s.id == d))
            .collect()
    }

    /// A journaled Agent Wire event.
    fn logged(&self, seq: u64) -> Result<&agent_wire_coordinate::Event, String> {
        (seq >= 1 && seq as usize <= self.wire_logged)
            .then(|| &self.wire.events()[seq as usize - 1])
            .ok_or_else(|| format!("no journaled agent wire event #{seq}"))
    }

    /// Whether anything can still move: a step out, or one that may still
    /// be assigned.
    fn moving(&self) -> bool {
        self.states.iter().enumerate().any(|(i, s)| match s {
            StepState::Running { .. } => true,
            StepState::Waiting => !self
                .deps(i)
                .into_iter()
                .any(|j| matches!(self.states[j], StepState::Failed(_) | StepState::Blocked(_))),
            _ => false,
        })
    }

    fn all_quests_complete(&self) -> bool {
        let quests = self.wire.quests();
        self.plan.steps.iter().all(|s| {
            quests
                .get(&self.quest(&s.id))
                .is_some_and(|q| q.state == QuestState::Complete)
        })
    }

    /// Apply one record: the only way state changes, live or on replay.
    fn apply(&mut self, record: &Record) -> Result<(), String> {
        match record {
            Record::TaskAccepted { .. } | Record::PlanValidated { .. } => {
                return Err("the task was already accepted".into());
            }
            Record::Wire { event } => {
                let due = self.wire_logged as u64 + 1;
                if event.seq != due {
                    return Err(format!(
                        "agent wire event #{} where #{due} was due",
                        event.seq
                    ));
                }
                match self.wire.events().get(self.wire_logged) {
                    // Live: Agent Wire already holds it.
                    Some(held) if held == event => {}
                    Some(_) => return Err("differs from agent wire's own event".into()),
                    // Replay: Agent Wire re-validates the whole history.
                    None => {
                        let mut events = self.wire.events().to_vec();
                        events.push(event.clone());
                        let workers = self.plan.workers().into_iter().map(WorkerId::new);
                        self.wire =
                            Orchestrator::from_events(self.wire.history().clone(), events, workers)
                                .map_err(|e| format!("agent wire refuses event #{due}: {e}"))?;
                    }
                }
                self.wire_logged += 1;
            }
            Record::StepAssigned {
                step,
                worker,
                request,
                wire_seq,
                at_ms,
                text,
            } => {
                let i = self.index(step)?;
                if self.states[i] != StepState::Waiting {
                    return Err(format!("{step} assigned twice"));
                }
                if *worker != self.plan.steps[i].worker {
                    return Err(format!("{step} is not {}'s", short(worker)));
                }
                if !self
                    .deps(i)
                    .into_iter()
                    .all(|d| self.states[d] == StepState::Complete)
                {
                    return Err(format!("{step} assigned before its dependencies"));
                }
                if self.requests.iter().flatten().any(|(r, _)| r == request) {
                    return Err(format!("request {request} reused"));
                }
                let quest = self.quest(step);
                match &self.logged(*wire_seq)?.kind {
                    EventKind::Assign { spec }
                        if spec.id == quest && spec.assignee.as_str() == worker => {}
                    _ => return Err(format!("event #{wire_seq} is not {step}'s ASSIGN")),
                }
                let s = &self.plan.steps[i];
                let mut fields = vec![("assign", s.capability.to_owned()), ("step", s.id.clone())];
                if !s.depends_on.is_empty() {
                    fields.push(("depends_on", s.depends_on.join(",")));
                }
                fields.push(("event", format!("ASSIGN #{wire_seq}")));
                self.trace
                    .push(trace(ORCHESTRATOR, Some(short(worker)), fields));
                self.states[i] = StepState::Running {
                    request: request.clone(),
                    since_ms: *at_ms,
                };
                self.requests[i] = Some((request.clone(), text.clone()));
            }
            Record::ReportReceived {
                step,
                report,
                from,
                status,
                excerpt,
                ..
            } => {
                let i = self.index(step)?;
                if !matches!(self.states[i], StepState::Running { .. }) {
                    return Err(format!("report for {step}, which is not out"));
                }
                if self.observed.contains(report) {
                    return Err(format!("report {report} taken in twice"));
                }
                if *from != self.plan.steps[i].worker {
                    return Err(format!("report for {step} from {}", short(from)));
                }
                let status =
                    parse_status(status).ok_or_else(|| format!("unknown status {status:?}"))?;
                // It must be what Agent Wire recorded from that worker.
                let quest = self.quest(step);
                let last = self.wire.events()[..self.wire_logged]
                    .iter()
                    .rev()
                    .find(|e| e.quest_id == quest)
                    .ok_or_else(|| format!("agent wire holds nothing for {step}"))?;
                let recorded = last.actor == Actor::Worker(WorkerId::new(from))
                    && match (&last.kind, status) {
                        (EventKind::Result { result }, Status::Ok) => {
                            result.artifacts.iter().any(|a| a.as_str() == report)
                        }
                        (EventKind::NeedHuman { .. }, Status::Refused | Status::Failed) => true,
                        _ => false,
                    };
                if !recorded {
                    return Err(format!("agent wire did not record report {report}"));
                }
                self.trace.push(trace(
                    short(from),
                    Some(ORCHESTRATOR),
                    vec![
                        ("report", step.clone()),
                        (
                            "status",
                            match status {
                                Status::Ok => "result".to_owned(),
                                other => format!("need_human · {}", other.as_str()),
                            },
                        ),
                    ],
                ));
                self.observed.insert(report.clone());
                self.reports[i] = Some(Report {
                    id: report.clone(),
                    from: from.clone(),
                    status,
                    text: excerpt.clone(),
                });
            }
            Record::ReportRejected {
                step,
                report,
                from,
                reason,
            } => {
                self.index(step)?;
                if !self.observed.insert(report.clone()) {
                    return Err(format!("report {report} taken in twice"));
                }
                self.trace.push(trace(
                    short(from),
                    Some(ORCHESTRATOR),
                    vec![("rejected", step.clone()), ("reason", reason.clone())],
                ));
            }
            Record::StepEvaluated { step, wire_seq } => {
                let i = self.index(step)?;
                if !matches!(self.states[i], StepState::Running { .. }) || self.reports[i].is_none()
                {
                    return Err(format!("{step} evaluated with no report"));
                }
                let quest = self.quest(step);
                let event = self.logged(*wire_seq)?;
                let completes = event.quest_id == quest
                    && event.actor == Actor::Orchestrator
                    && matches!(event.kind, EventKind::Complete { .. });
                let held = self
                    .wire
                    .quests()
                    .get(&quest)
                    .is_some_and(|q| q.state == QuestState::Complete);
                if !(completes && held) {
                    return Err(format!("agent wire does not hold {step} COMPLETE"));
                }
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("evaluate", format!("{step} · gates met · COMPLETE"))],
                ));
                self.states[i] = StepState::Complete;
            }
            Record::StepFailed { step, reason, .. } => {
                let i = self.index(step)?;
                if !matches!(
                    self.states[i],
                    StepState::Waiting | StepState::Running { .. }
                ) {
                    return Err(format!("{step} failed after it ended"));
                }
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("failed", step.clone()), ("reason", reason.clone())],
                ));
                self.states[i] = StepState::Failed(reason.clone());
            }
            Record::StepBlocked { step, reason } => {
                let i = self.index(step)?;
                let dep_failed = self.deps(i).into_iter().any(|d| {
                    matches!(self.states[d], StepState::Failed(_) | StepState::Blocked(_))
                });
                if self.states[i] != StepState::Waiting || !dep_failed {
                    return Err(format!("{step} blocked with no failed dependency"));
                }
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("blocked", step.clone()), ("reason", reason.clone())],
                ));
                self.states[i] = StepState::Blocked(reason.clone());
            }
            Record::TaskCompleted { at_ms } => {
                if self.outcome.is_some() || self.moving() || !self.all_quests_complete() {
                    return Err("completed without agent wire holding every quest COMPLETE".into());
                }
                self.trace
                    .push(trace(ORCHESTRATOR, None, vec![("evaluate", "pass".into())]));
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("complete", self.id.clone())],
                ));
                self.outcome = Some(Outcome::Complete);
                self.terminal_at_ms = Some(*at_ms);
            }
            Record::TaskFailed { reason, at_ms } => {
                if self.outcome.is_some() || self.moving() {
                    return Err("failed while the task could still move".into());
                }
                self.trace.push(trace(
                    ORCHESTRATOR,
                    None,
                    vec![("evaluate", format!("fail · {reason}"))],
                ));
                self.outcome = Some(Outcome::Failed(reason.clone()));
                self.terminal_at_ms = Some(*at_ms);
            }
            Record::Announced { message } => {
                if self.outcome.is_none() || self.announced.is_some() {
                    return Err("announced twice, or before an outcome".into());
                }
                self.announced = Some(message.clone());
            }
        }
        Ok(())
    }

    /// Apply a live transition and keep it for the journal.
    fn commit(&mut self, record: Record) {
        self.apply(&record).expect("a live transition is legal");
        self.journal.push(record);
    }

    /// Journal every Agent Wire event not journaled yet.
    fn log_wire(&mut self) {
        while let Some(event) = self.wire.events().get(self.wire_logged).cloned() {
            self.commit(Record::Wire { event });
        }
    }

    /// Assign every step whose dependencies are all complete, and block
    /// every step one of whose dependencies failed. `new_id` names each
    /// room request. Independent steps come back together.
    ///
    /// The validated plan is history, not a grant: each assignment is
    /// authorized against `ctx` — trust, room, policy and authority as
    /// they are now. A step that is no longer permitted is refused and
    /// fails, durably; it is never sent.
    pub fn ready(
        &mut self,
        now_ms: u64,
        ctx: &Context,
        new_id: impl FnMut() -> String,
    ) -> Vec<Dispatch> {
        self.ready_in(&REGISTRY, now_ms, ctx, new_id)
    }

    /// [`TaskRun::ready`] against a given registry.
    pub fn ready_in(
        &mut self,
        registry: &[Capability],
        now_ms: u64,
        ctx: &Context,
        mut new_id: impl FnMut() -> String,
    ) -> Vec<Dispatch> {
        if self.outcome.is_some() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for i in 0..self.plan.steps.len() {
            if self.states[i] != StepState::Waiting {
                continue;
            }
            let step = self.plan.steps[i].clone();
            let deps = self.deps(i);
            if let Some(&bad) = deps
                .iter()
                .find(|&&d| matches!(self.states[d], StepState::Failed(_) | StepState::Blocked(_)))
            {
                let reason = format!("{} did not complete", self.plan.steps[bad].id);
                self.commit(Record::StepBlocked {
                    step: step.id.clone(),
                    reason,
                });
                continue;
            }
            if !deps.iter().all(|&d| self.states[d] == StepState::Complete) {
                continue;
            }
            if let Err(reason) = authorize(registry, &step, ctx) {
                self.fail(i, reason, now_ms);
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
                    self.fail(i, format!("agent wire refused assign: {e}"), now_ms);
                    continue;
                }
            };
            self.log_wire();
            let request = new_id();
            let text = format!("task:: {} · {}\n{text}", self.id, step.id);
            self.commit(Record::StepAssigned {
                step: step.id.clone(),
                worker: step.worker.clone(),
                request: request.clone(),
                wire_seq: seq,
                at_ms: now_ms,
                text: text.clone(),
            });
            out.push(Dispatch {
                step: step.id.clone(),
                worker: step.worker.clone(),
                request,
                text,
            });
        }
        self.settle(now_ms);
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
                r.text
            ));
        }
        text.push_str(&format!("synthesize:: {}", step.objective));
        text
    }

    /// Take in a room report. Only a report replying to a running step's
    /// request counts, and Agent Wire records it only if it comes from that
    /// step's assignee. A report already taken in changes nothing. Returns
    /// whether anything changed.
    pub fn observe(&mut self, from: &str, report: &RoomMessage, now_ms: u64) -> bool {
        if self.outcome.is_some()
            || report.kind != Kind::Report
            || self.observed.contains(&report.id)
        {
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
        // ACCEPT may have been recorded before RESULT was refused.
        self.log_wire();
        if let Err(e) = recorded {
            // Not the assignee, or not a legal report: Agent Wire records
            // nothing and the step keeps waiting for its real worker.
            let reason = match e {
                CoordinationError::NotAuthorized { .. } => "not the assignee".to_owned(),
                other => other.to_string(),
            };
            self.commit(Record::ReportRejected {
                step: step.id.clone(),
                report: report.id.clone(),
                from: from.to_owned(),
                reason,
            });
            return true;
        }
        self.commit(Record::ReportReceived {
            step: step.id.clone(),
            report: report.id.clone(),
            from: from.to_owned(),
            status: status.as_str().to_owned(),
            excerpt: deaddrop_klodik::model::cut(report.text.trim(), MAX_REPORT_CHARS),
            at_ms: now_ms,
        });
        if status != Status::Ok {
            self.fail(i, format!("{} {}", short(from), status.as_str()), now_ms);
            return true;
        }
        let verdict = self.wire.evaluate(&quest);
        self.log_wire();
        match verdict {
            Ok(Verdict::Complete) => self.commit(Record::StepEvaluated {
                step: step.id.clone(),
                wire_seq: self.wire.events().len() as u64,
            }),
            Ok(Verdict::RevisionRequired { unmet_gates }) => {
                let unmet: Vec<&str> = unmet_gates.iter().map(|g| g.as_str()).collect();
                // V0 never revises or replans: an unmet gate fails the step.
                self.fail(i, format!("gates unmet: {}", unmet.join(",")), now_ms);
            }
            Err(e) => self.fail(i, format!("agent wire: {e}"), now_ms),
        }
        self.settle(now_ms);
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
                now_ms,
            );
        }
        if !late.is_empty() {
            self.settle(now_ms);
        }
        !late.is_empty()
    }

    fn fail(&mut self, i: usize, reason: String, at_ms: u64) {
        self.commit(Record::StepFailed {
            step: self.plan.steps[i].id.clone(),
            reason,
            at_ms,
        });
    }

    /// Decide the task, once, when nothing can move any more. Complete only
    /// if Agent Wire itself holds every step's quest COMPLETE.
    fn settle(&mut self, now_ms: u64) {
        if self.outcome.is_some() || self.moving() {
            return;
        }
        if self.all_quests_complete() {
            self.commit(Record::TaskCompleted { at_ms: now_ms });
        } else {
            let failed: Vec<String> = self
                .plan
                .steps
                .iter()
                .zip(&self.states)
                .filter_map(|(s, st)| match st {
                    StepState::Failed(why) => Some(format!("{} failed ({why})", s.id)),
                    StepState::Blocked(_) | StepState::Waiting => Some(format!("{} blocked", s.id)),
                    _ => None,
                })
                .collect();
            self.commit(Record::TaskFailed {
                reason: failed.join("; "),
                at_ms: now_ms,
            });
        }
    }

    /// The outcome line the wire sends, once there is an outcome.
    pub fn outcome_text(&self) -> Option<String> {
        let id = &self.id;
        Some(match self.outcome.as_ref()? {
            Outcome::Complete => {
                let by = self.result().map(|r| short(&r.from)).unwrap_or_default();
                format!("task:: complete\nid:: {id}\nresult:: the {by} report above")
            }
            Outcome::Failed(why) => format!("task:: failed\nid:: {id}\nreason:: {why}"),
        })
    }

    /// Record that the outcome line goes out as room message `message`.
    /// Only once: returns false if there is no outcome or it was announced.
    pub fn announce(&mut self, message: &str) -> bool {
        if self.outcome.is_none() || self.announced.is_some() {
            return false;
        }
        self.commit(Record::Announced {
            message: message.to_owned(),
        });
        true
    }

    /// Every room message the journal says this task sends, in order, with
    /// its id: your task, the acceptance, each step request, the outcome.
    pub fn messages(&self) -> Vec<Message> {
        let message = |id: &str, kind, mentions, text, wire| Message {
            id: id.to_owned(),
            kind,
            mentions,
            text,
            wire,
        };
        let a = &self.accepted;
        let mut out = vec![
            message(
                &a.human_message,
                Kind::Message,
                Vec::new(),
                format!("{}\n\n/task::wire", a.objective.trim()),
                false,
            ),
            message(
                &a.accepted_message,
                Kind::Message,
                Vec::new(),
                format!(
                    "task:: accepted\nid:: {}\nsteps:: {}",
                    self.id,
                    self.plan.steps.len()
                ),
                true,
            ),
        ];
        // In the order they were assigned.
        for record in &self.journal {
            if let Record::StepAssigned {
                worker,
                request,
                text,
                ..
            } = record
            {
                out.push(message(
                    request,
                    Kind::Request,
                    vec![worker.clone()],
                    text.clone(),
                    true,
                ));
            }
        }
        if let (Some(id), Some(text)) = (&self.announced, self.outcome_text()) {
            out.push(message(id, Kind::Message, Vec::new(), text, true));
        }
        out
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
pub(crate) mod tests {
    use super::*;
    use crate::plan::tests::{TASK, ctx, golden};
    use crate::plan::validate;
    use agent_wire_coordinate::EventKind;
    use deaddrop_room::Kind;

    const RESEARCH: &str = "research:agent:deaddrop";
    const GITHUB: &str = "github:agent:deaddrop";
    const KLODIK: &str = "klodik:agent:deaddrop";

    fn start() -> (TaskRun, Vec<Dispatch>) {
        let mut run = TaskRun::accept(
            "T-1042",
            accepted(),
            validate(TASK, &golden(), &ctx()).unwrap(),
        );
        let mut n = 0;
        let sent = run.ready(0, &ctx(), || {
            n += 1;
            format!("rq{n}")
        });
        (run, sent)
    }

    pub(crate) fn accepted() -> Accepted {
        Accepted {
            room: "d34ddr0p".into(),
            objective: TASK.into(),
            human_message: "human-1".into(),
            accepted_message: "accepted-1".into(),
            at_ms: 1_000,
        }
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
        run.ready(1, &ctx(), || "rq-next".into())
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
        assert!(run.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5));
        assert!(next(&mut run).is_empty(), "synthesis waits for research");
        assert_eq!(run.outcome, None);
        assert!(run.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND), 5));
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
            &report(&synth[0], Status::Ok, "Both: 1.99.0, and yes."),
            5
        ));
        assert_eq!(run.outcome, Some(Outcome::Complete));
        assert_eq!(run.result().unwrap().text, "Both: 1.99.0, and yes.");
    }

    #[test]
    fn agent_wire_holds_the_lifecycle_and_completes_exactly_once() {
        let (mut run, sent) = start();
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5);
        run.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND), 5);
        let synth = next(&mut run);
        run.observe(KLODIK, &report(&synth[0], Status::Ok, "done"), 5);
        // Replays change nothing.
        assert!(!run.observe(KLODIK, &report(&synth[0], Status::Ok, "done"), 5));
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
        assert!(run.observe(KLODIK, &report(&sent[1], Status::Ok, YES), 5));
        assert!(matches!(run.states[1], StepState::Running { .. }));
        assert_eq!(
            run.trace.last().unwrap().fields[1],
            ("reason", "not the assignee".into())
        );
        // A worker's message claiming completion is just a message.
        let mut claim = report(&sent[0], Status::Ok, "task:: complete");
        claim.kind = Kind::Message;
        assert!(!run.observe(RESEARCH, &claim, 5));
        assert_eq!(run.outcome, None);
    }

    #[test]
    fn a_failure_keeps_other_reports_and_blocks_what_depends_on_it() {
        let (mut run, sent) = start();
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5);
        run.observe(
            RESEARCH,
            &report(&sent[0], Status::Failed, "research failed: timeout"),
            5,
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
            5,
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
        run.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5);
        assert!(!run.expire(STEP_TIMEOUT_MS));
        assert!(run.expire(STEP_TIMEOUT_MS + 1));
        assert!(matches!(&run.outcome, Some(Outcome::Failed(w)) if w.contains("timeout")));
    }

    // --- Durable journal: replay, restart, idempotency. ---

    /// A driver like the TUI's: after every pass, what is new in the run is
    /// committed to `disk` (one commit per pass) before anything is sent.
    struct Driver {
        disk: Vec<Vec<Record>>,
        /// Reports workers have sent, ever (signed history survives).
        delivered: Vec<(String, RoomMessage)>,
        /// Room requests ever handed to the transport.
        sent: Vec<String>,
        n: usize,
    }

    impl Driver {
        fn persist(&mut self, run: &TaskRun) {
            let on_disk: usize = self.disk.iter().map(Vec::len).sum();
            let new = run.journal()[on_disk..].to_vec();
            if !new.is_empty() {
                self.disk.push(new);
            }
        }

        fn pass(&mut self, run: &mut TaskRun) {
            for (from, m) in self.delivered.clone() {
                run.observe(&from, &m, 10);
            }
            run.expire(10);
            let mut n = self.n;
            let ready = run.ready(10, &ctx(), || {
                n += 1;
                format!("rq{n}")
            });
            self.n = n;
            if run.outcome.is_some() && run.announced().is_none() {
                run.announce("done-1");
            }
            self.persist(run);
            // Sent only after the commit.
            self.sent.extend(ready.into_iter().map(|d| d.request));
        }

        /// The first running step in `order` answers its request.
        fn answer(&mut self, run: &TaskRun, order: &[&str]) {
            for step in order {
                let i = run.plan.steps.iter().position(|s| s.id == *step).unwrap();
                let Some(request) = run.request(i) else {
                    continue;
                };
                let id = format!("re-{request}");
                if self.delivered.iter().any(|(_, m)| m.id == id) {
                    continue;
                }
                let s = &run.plan.steps[i];
                let text = match s.capability {
                    WEB_SEARCH => FOUND,
                    GITHUB_INSPECT => YES,
                    _ => "Both: 1.99.0, and yes.",
                };
                let m = RoomMessage {
                    room: "d34ddr0p".into(),
                    id,
                    kind: Kind::Report,
                    hop: 0,
                    mentions: vec![],
                    reply_to: Some(request.to_owned()),
                    status: Some(Status::Ok),
                    text: text.into(),
                };
                self.delivered.push((s.worker.clone(), m));
                return;
            }
        }
    }

    const ORDER: [&str; 3] = ["inspect_commit", "research_release", "synthesize"];

    fn flat(disk: &[Vec<Record>]) -> Vec<Record> {
        disk.iter().flatten().cloned().collect()
    }

    fn count(records: &[Record], f: impl Fn(&Record) -> bool) -> usize {
        records.iter().filter(|r| f(r)).count()
    }

    /// Run to the end, crashing after commit `crash` (if any): the run is
    /// then rebuilt from disk alone, workers that were asked may answer
    /// while it is down, and it carries on.
    fn run_with_crash(crash: Option<usize>) -> (TaskRun, Driver) {
        let mut d = Driver {
            disk: Vec::new(),
            delivered: Vec::new(),
            sent: Vec::new(),
            n: 0,
        };
        let mut run = TaskRun::accept(
            "T-1042",
            accepted(),
            validate(TASK, &golden(), &ctx()).unwrap(),
        );
        d.persist(&run);
        let mut crashed = false;
        for _ in 0..20 {
            if !crashed && Some(d.disk.len()) == crash {
                crashed = true;
                let before = run.journal().to_vec();
                // Down: whatever was asked may be answered meanwhile.
                d.answer(&run, &ORDER);
                run = TaskRun::replay("T-1042", &flat(&d.disk)).unwrap();
                assert_eq!(run.journal(), before.as_slice(), "replay is exact");
            }
            if run.announced().is_some() {
                break;
            }
            d.pass(&mut run);
            d.answer(&run, &ORDER);
        }
        (run, d)
    }

    #[test]
    fn a_restart_at_any_commit_reruns_nothing_and_completes_once() {
        let (clean, clean_d) = run_with_crash(None);
        let commits = clean_d.disk.len();
        assert!(commits >= 5, "{commits}");
        for crash in 1..=commits {
            let (run, d) = run_with_crash(Some(crash));
            let records = flat(&d.disk);
            assert_eq!(run.outcome, Some(Outcome::Complete), "crash {crash}");
            for step in ORDER {
                assert_eq!(
                    count(
                        &records,
                        |r| matches!(r, Record::StepAssigned { step: s, .. } if s == step)
                    ),
                    1,
                    "crash {crash}: {step} assigned once"
                );
                assert_eq!(
                    count(
                        &records,
                        |r| matches!(r, Record::ReportReceived { step: s, .. } if s == step)
                    ),
                    1,
                    "crash {crash}: {step} reported once"
                );
            }
            assert_eq!(d.sent.len(), 3, "crash {crash}: each worker asked once");
            assert_eq!(d.delivered.len(), 3, "crash {crash}: each worker ran once");
            assert_eq!(
                count(&records, |r| matches!(r, Record::TaskCompleted { .. })),
                1
            );
            assert_eq!(
                count(&records, |r| matches!(r, Record::Announced { .. })),
                1
            );
            // The same history as a run that never stopped.
            assert_eq!(run.trace.len(), clean.trace.len(), "crash {crash}");
            assert_eq!(run.wire().events().len(), clean.wire().events().len());
        }
    }

    #[test]
    fn replay_of_every_prefix_is_deterministic() {
        let (_, d) = run_with_crash(None);
        let all = flat(&d.disk);
        for k in 2..=all.len() {
            let a = TaskRun::replay("T-1042", &all[..k]).unwrap();
            let b = TaskRun::replay("T-1042", &all[..k]).unwrap();
            assert_eq!(a.states, b.states);
            assert_eq!(a.trace, b.trace);
            assert_eq!(a.wire().events(), b.wire().events());
            assert_eq!(a.journal(), &all[..k]);
        }
    }

    #[test]
    fn github_done_research_pending_survives_a_restart() {
        // Commits: accept, assign both, github's report. Crash there.
        let (run, d) = run_with_crash(Some(3));
        assert_eq!(run.outcome, Some(Outcome::Complete));
        let records = flat(&d.disk);
        let synth = records
            .iter()
            .position(|r| matches!(r, Record::StepAssigned { step, .. } if step == "synthesize"))
            .unwrap();
        let research = records
            .iter()
            .position(
                |r| matches!(r, Record::StepEvaluated { step, .. } if step == "research_release"),
            )
            .unwrap();
        assert!(research < synth, "synthesis waited for research");
    }

    #[test]
    fn replayed_reports_and_completion_change_nothing() {
        let (mut run, d) = run_with_crash(None);
        let before = run.journal().len();
        for (from, m) in &d.delivered {
            assert!(!run.observe(from, m, 99));
        }
        assert!(run.ready(99, &ctx(), || "x".into()).is_empty());
        assert!(!run.expire(u64::MAX));
        assert!(!run.announce("again"));
        assert_eq!(run.journal().len(), before);
    }

    fn tampered(f: impl Fn(&mut Vec<Record>)) -> String {
        let (_, d) = run_with_crash(None);
        let mut all = flat(&d.disk);
        f(&mut all);
        TaskRun::replay("T-1042", &all).unwrap_err().to_string()
    }

    #[test]
    fn a_journal_that_is_not_a_legal_history_is_refused() {
        // A second COMPLETE.
        assert!(
            tampered(|r| {
                let last = r
                    .iter()
                    .rposition(|x| matches!(x, Record::TaskCompleted { .. }))
                    .unwrap();
                r.insert(last + 1, r[last].clone());
            })
            .contains("completed without")
        );
        // Completion claimed where Agent Wire holds less.
        assert!(tampered(|r| {
            r.retain(|x| !matches!(x, Record::Wire { event } if event.kind.name() == "COMPLETE" && event.quest_id.as_str().ends_with("synthesize")));
        })
        .contains("agent wire"));
        // A worker's event dressed up as the orchestrator's COMPLETE.
        assert!(
            tampered(|r| {
                for x in r.iter_mut() {
                    if let Record::Wire { event } = x
                        && event.kind.name() == "COMPLETE"
                    {
                        event.actor = agent_wire_coordinate::Actor::Worker(WorkerId::new(RESEARCH));
                        break;
                    }
                }
            })
            .contains("agent wire refuses")
        );
        // A capability routed to another peer: the journal grants nothing.
        assert!(
            tampered(|r| {
                if let Record::PlanValidated { steps } = &mut r[1] {
                    steps[0].worker = KLODIK.into();
                }
            })
            .contains("is not served by")
        );
        // The same report taken in twice.
        assert!(
            tampered(|r| {
                let i = r
                    .iter()
                    .position(|x| matches!(x, Record::ReportReceived { .. }))
                    .unwrap();
                r.insert(i + 1, r[i].clone());
            })
            .contains("record")
        );
        // A step assigned twice.
        assert!(
            tampered(|r| {
                let i = r
                    .iter()
                    .position(|x| matches!(x, Record::StepAssigned { .. }))
                    .unwrap();
                r.push(r[i].clone());
            })
            .contains("assigned twice")
        );
    }

    // --- Dispatch authorization: the plan is history, not a grant. ---

    /// Research and GitHub reported and are COMPLETE; synthesis not yet
    /// assigned. Then `ready` runs under `now`.
    fn both_reported_then(now: &Context) -> (TaskRun, Vec<Dispatch>) {
        let (mut run, sent) = start();
        let quiet = |run: &mut TaskRun, d: &Dispatch, who: &str, text: &str| {
            assert!(run.observe(who, &report(d, Status::Ok, text), 5));
        };
        quiet(&mut run, &sent[1], GITHUB, YES);
        quiet(&mut run, &sent[0], RESEARCH, FOUND);
        assert_eq!(run.states[..2], [StepState::Complete, StepState::Complete]);
        assert_eq!(
            run.states[2],
            StepState::Waiting,
            "synthesis not yet assigned"
        );
        let synth = run.ready(6, now, || "rq-synth".into());
        (run, synth)
    }

    fn without(ctx: Context, field: fn(&mut Context) -> &mut Vec<String>, who: &str) -> Context {
        let mut ctx = ctx;
        field(&mut ctx).retain(|p| p != who);
        ctx
    }

    fn refused(run: &TaskRun, synth: &[Dispatch], reason: &str) {
        assert!(synth.is_empty(), "no synthesis request");
        assert_eq!(run.states[2], StepState::Failed(reason.into()));
        assert_eq!(
            run.outcome,
            Some(Outcome::Failed(format!("synthesize failed ({reason})")))
        );
        assert!(
            !run.wire()
                .events()
                .iter()
                .any(|e| e.quest_id.as_str().ends_with("synthesize")),
            "agent wire never assigned it"
        );
        assert!(
            !run.journal()
                .iter()
                .any(|r| matches!(r, Record::TaskCompleted { .. }))
        );
        assert_eq!(
            run.reports[0].as_ref().unwrap().text.lines().next(),
            Some("Rust 1.99.0, October 1, 2026.")
        );
        assert_eq!(run.reports[1].as_ref().unwrap().text, YES, "history stands");
        // Durable and deterministic: the refusal replays as it happened.
        let again = TaskRun::replay(&run.id, run.journal()).unwrap();
        assert_eq!(again.states, run.states);
        assert_eq!(again.outcome, run.outcome);
        assert_eq!(again.trace, run.trace);
    }

    #[test]
    fn current_policy_decides_dispatch_not_the_accepted_plan() {
        let now = without(ctx(), |c| &mut c.may_ask, KLODIK);
        let (run, synth) = both_reported_then(&now);
        refused(
            &run,
            &synth,
            "current policy no longer permits synthesize via klodik",
        );
        // Still permitted: dispatched as before.
        let (_, synth) = both_reported_then(&ctx());
        assert_eq!(synth.len(), 1);
    }

    #[test]
    fn revoked_trust_or_membership_refuses_dispatch() {
        let (run, synth) = both_reported_then(&without(ctx(), |c| &mut c.trusted, KLODIK));
        refused(&run, &synth, "klodik is no longer a trusted peer");
        let (run, synth) = both_reported_then(&without(ctx(), |c| &mut c.members, KLODIK));
        refused(&run, &synth, "klodik is no longer in the room");
    }

    #[test]
    fn capability_alone_never_wins() {
        // The registry still has `synthesize`, but serves it from another
        // peer, or with other authority than the plan was accepted with.
        let mut moved = REGISTRY;
        moved[2].peer = GITHUB;
        let (mut a, sent) = start();
        a.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5);
        a.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND), 5);
        let mut b = a.clone();
        let synth = a.ready_in(&moved, 6, &ctx(), || "rq".into());
        refused(&a, &synth, "synthesize is no longer served by klodik");
        let mut regraded = REGISTRY;
        regraded[2].mode = Mode::ReadOnly;
        let synth = b.ready_in(&regraded, 6, &ctx(), || "rq".into());
        refused(
            &b,
            &synth,
            "synthesize authority changed (report-only → read-only)",
        );
    }

    #[test]
    fn an_assignment_made_before_revocation_is_still_reconciled() {
        // Research and Klodik are revoked after Research was assigned.
        let (mut run, sent) = start();
        let now = without(
            without(ctx(), |c| &mut c.may_ask, RESEARCH),
            |c| &mut c.may_ask,
            KLODIK,
        );
        assert!(run.ready(5, &now, || "never".into()).is_empty());
        // Its report is history: taken in once, evaluated by Agent Wire.
        assert!(run.observe(RESEARCH, &report(&sent[0], Status::Ok, FOUND), 5));
        assert!(run.observe(GITHUB, &report(&sent[1], Status::Ok, YES), 5));
        assert_eq!(run.states[0], StepState::Complete);
        let synth = run.ready(6, &now, || "rq".into());
        refused(
            &run,
            &synth,
            "current policy no longer permits synthesize via klodik",
        );
        let assigned = run
            .journal()
            .iter()
            .filter(|r| matches!(r, Record::StepAssigned { .. }))
            .count();
        assert_eq!(assigned, 2, "no second assignment, no new one");
    }
}
