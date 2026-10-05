//! `/task::wire` in a room: the TUI side of `deaddrop-task`.
//!
//! The parser decides a draft is a task; the planner (off the UI thread)
//! proposes; `deaddrop_task::plan::validate` decides; a live task runs as a
//! `deaddrop_task::run::TaskRun`, whose Agent Wire history decides each
//! step and the task. This module only carries messages: it turns
//! dispatches into ordinary room requests, feeds room reports back in, and
//! says what happened. A dry run sends nothing at all.
//!
//! Durable: with a node home, every task has a journal
//! (`deaddrop_task::journal`) and each transition is committed to it before
//! anything it implies is sent. On start every journal is replayed; a task
//! that was running carries on from exactly its journaled state, and any
//! message the journal says was sent but signed history does not hold is
//! sent again with its original id. Nothing is rerun because the UI
//! restarted. A terminal task's episode (`deaddrop_task::memory`) is derived
//! from its journal.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use deaddrop_room::{Kind, RoomMessage, short};
use deaddrop_task::journal::{self, Fault, Journal};
use deaddrop_task::memory::{self, Episode};
use deaddrop_task::plan::{Plan, Proposed};
use deaddrop_task::registry::Context;
use deaddrop_task::run::{Accepted, TaskRun, Trace};

use super::{Action, App, RoomOutgoing, Row};

/// A task the planner is working on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planning {
    pub id: String,
    pub room: String,
    pub payload: String,
    pub dry_run: bool,
    pub since: Instant,
}

/// What the caller hands the planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRequest {
    pub id: String,
    pub payload: String,
}

/// Local lines in a room, shown to you only: dry runs and refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub room: String,
    pub lines: Vec<String>,
}

/// A task that never ran (dry run, or refused), for the wire view.
#[derive(Debug, Clone)]
pub struct Logged {
    pub room: String,
    pub id: String,
    pub trace: Vec<Trace>,
}

#[derive(Debug)]
pub struct Live {
    pub room: String,
    pub run: TaskRun,
    /// Its journal; `None` only without a node home (tests).
    journal: Option<Journal>,
    /// Replayed from a journal: check signed history for anything the
    /// journal sent that never left, once, after the first sync.
    resume: bool,
    /// The journal could not be written: the task stops here, visibly.
    halted: Option<String>,
    /// Its episode is kept.
    remembered: bool,
}

#[derive(Debug, Default)]
pub struct Tasks {
    pub planning: Option<Planning>,
    pub live: Vec<Live>,
    pub notes: Vec<Note>,
    pub logged: Vec<Logged>,
    outbox: Vec<RoomOutgoing>,
    /// Room message ids the task layer wrote: lifecycle events and step
    /// requests, this session and, from the journals, every earlier one.
    /// Shown as `λ wire`, never as `you`.
    generated: HashSet<String>,
    /// The node home whose `tasks/` and `memory/` this uses.
    home: Option<PathBuf>,
    /// Injected journal failure (tests only).
    fault: Option<Fault>,
}

/// Wall clock, in ms: step timeouts and journal times survive a restart.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Commit what `live.run` has not journaled yet. `false`: it could not,
/// and the task is halted; nothing it implies may be sent.
///
/// Persist before act: on failure the run is rebuilt from the durable
/// prefix alone, so no state the journal does not hold — an assignment, a
/// completion, an episode — survives in memory either.
fn commit(live: &mut Live) -> bool {
    if live.halted.is_some() {
        return false;
    }
    let Some(journal) = &mut live.journal else {
        return true;
    };
    let Err(e) = journal.catch_up(live.run.journal()) else {
        return true;
    };
    let durable = &live.run.journal()[..journal.written()];
    live.run = TaskRun::replay(&live.run.id, durable).expect("the durable prefix replays");
    live.halted = Some(format!("journal write failed ({e}); nothing further sent"));
    false
}

/// The plan, as a person reads it: steps, dependencies, workers.
fn describe(plan: &Plan) -> Vec<String> {
    let mut lines = vec!["plan::".to_owned()];
    for s in &plan.steps {
        lines.push(format!(
            "  {}  {} → {}",
            s.id,
            s.capability,
            short(&s.worker)
        ));
    }
    let deps: Vec<String> = plan
        .steps
        .iter()
        .filter(|s| !s.depends_on.is_empty())
        .map(|s| format!("  {} <- {}", s.id, s.depends_on.join(", ")))
        .collect();
    if !deps.is_empty() {
        lines.push("dependencies::".into());
        lines.extend(deps);
    }
    let workers: Vec<&str> = plan.workers().into_iter().map(short).collect();
    lines.push(format!("workers:: {}", workers.join(", ")));
    lines
}

fn trace(fields: Vec<(&'static str, String)>) -> Trace {
    Trace {
        from: "orchestrator".into(),
        to: None,
        fields,
    }
}

impl App {
    /// Enter on a `/task::wire` draft in a room. Plans first; nothing is
    /// asked of anyone until the plan is validated.
    pub(super) fn start_task(&mut self, payload: String, dry_run: bool) -> Action {
        let Some(room) = self
            .compose
            .as_ref()
            .and_then(|c| c.to.strip_prefix('#'))
            .map(str::to_owned)
        else {
            self.status = "wire: /task::wire runs in a room".into();
            return Action::None;
        };
        if self.tasks.planning.is_some() {
            self.status = "wire: a task is already being planned".into();
            return Action::None;
        }
        if !dry_run
            && self
                .tasks
                .live
                .iter()
                .any(|l| l.room == room && l.run.outcome.is_none())
        {
            self.status = "wire: a task is already running here (V0: one at a time)".into();
            return Action::None;
        }
        let id = deaddrop_task::task_id(&deaddrop_protocol::MessageId::generate().to_string());
        self.tasks.planning = Some(Planning {
            id: id.clone(),
            room,
            payload: payload.clone(),
            dry_run,
            since: Instant::now(),
        });
        if let Some(compose) = &mut self.compose {
            compose.draft.clear();
            compose.cursor = 0;
        }
        self.follow = true;
        self.status = format!(
            "wire: task {id} · planning{}",
            if dry_run { " · dry run" } else { "" }
        );
        Action::PlanTask(TaskRequest { id, payload })
    }

    /// What this node knows for resolving workers in `room`.
    fn task_context(&self, room: &str) -> Context {
        Context {
            trusted: self.peers().iter().map(|p| p.id.clone()).collect(),
            members: self
                .rooms
                .iter()
                .find(|r| r.name == room)
                .map(|r| r.members.clone())
                .unwrap_or_default(),
            may_ask: self.task_policy.clone(),
        }
    }

    /// The planner's answer. Validated here, deterministically; a dry run
    /// stops at the plan, a live task starts.
    pub fn planned(&mut self, id: &str, proposed: Result<Proposed, String>) {
        let Some(planning) = self.tasks.planning.take_if(|p| p.id == id) else {
            return;
        };
        let Planning {
            room,
            payload,
            dry_run,
            ..
        } = planning;
        let validated = proposed
            .and_then(|p| deaddrop_task::plan::validate(&payload, &p, &self.task_context(&room)));
        let plan = match validated {
            Ok(plan) => plan,
            Err(why) => {
                self.status = format!("wire: task {id} rejected · {why}");
                self.tasks.notes.push(Note {
                    room: room.clone(),
                    lines: vec![format!("task:: rejected · {id}"), format!("reason:: {why}")],
                });
                self.tasks.logged.push(Logged {
                    room,
                    id: id.to_owned(),
                    trace: vec![trace(vec![("plan", format!("rejected · {why}"))])],
                });
                return;
            }
        };
        if dry_run {
            let mut lines = vec![format!("task:: dry-run · {id} · nothing was run")];
            lines.extend(describe(&plan));
            self.tasks.notes.push(Note {
                room: room.clone(),
                lines,
            });
            let mut log = vec![trace(vec![(
                "plan",
                format!("valid · {} steps · dry run, not run", plan.steps.len()),
            )])];
            for s in &plan.steps {
                let mut fields = vec![
                    ("would assign", s.capability.to_owned()),
                    ("step", s.id.clone()),
                ];
                if !s.depends_on.is_empty() {
                    fields.push(("depends_on", s.depends_on.join(",")));
                }
                log.push(Trace {
                    from: "orchestrator".into(),
                    to: Some(short(&s.worker).to_owned()),
                    fields,
                });
            }
            self.tasks.logged.push(Logged {
                room,
                id: id.to_owned(),
                trace: log,
            });
            self.status = format!("wire: task {id} · dry run · plan valid · nothing run");
            return;
        }
        let steps = plan.steps.len();
        let fresh = || deaddrop_protocol::MessageId::generate().to_string();
        let accepted = Accepted {
            room: room.clone(),
            objective: payload.clone(),
            human_message: fresh(),
            accepted_message: fresh(),
            at_ms: now_ms(),
        };
        let run = TaskRun::accept(id, accepted, plan);
        // Journaled before anything is sent; no journal, no task.
        let journal = match &self.tasks.home {
            Some(home) => {
                match Journal::create_with(home, id, run.journal(), self.tasks.fault.clone()) {
                    Ok(j) => Some(j),
                    Err(e) => {
                        let why = format!("journal: {e}");
                        self.status = format!("wire: task {id} not started · {why}");
                        self.tasks.notes.push(Note {
                            room,
                            lines: vec![
                                format!("task:: not started · {id}"),
                                format!("reason:: {why}"),
                            ],
                        });
                        return;
                    }
                }
            }
            None => None,
        };
        // What you wrote, as you wrote it: yours. That it was accepted: the
        // wire's, not yours.
        for m in run.messages() {
            self.queue(&room, m);
        }
        self.tasks.live.push(Live {
            room,
            run,
            journal,
            resume: false,
            halted: None,
            remembered: false,
        });
        self.status = format!("wire: task {id} accepted · {steps} steps");
        self.advance_tasks();
    }

    /// Replay every task journal under `home`, and use it from now on.
    /// A task that cannot be replayed is not resumed and says why.
    pub fn load_tasks(&mut self, home: &Path) {
        self.tasks.home = Some(home.to_owned());
        let journals = match journal::list(home) {
            Ok(j) => j,
            Err(e) => {
                self.status = format!("wire: task journals: {e}");
                return;
            }
        };
        let mut problems = Vec::new();
        for (task, path) in journals {
            let opened = Journal::open(&path, &task).and_then(|(journal, loaded)| {
                TaskRun::replay(&task, &loaded.records).map(|run| (journal, loaded.torn, run))
            });
            let (journal, torn, run) = match opened {
                Ok(o) => o,
                Err(e) => {
                    problems.push(format!("{task}: {e} · not resumed"));
                    continue;
                }
            };
            if torn {
                problems.push(format!("{task}: torn final commit discarded"));
            }
            for m in run.messages().into_iter().filter(|m| m.wire) {
                self.tasks.generated.insert(m.id);
            }
            self.tasks.live.push(Live {
                room: run.accepted.room.clone(),
                // Even a finished task: its outcome line may never have left.
                resume: true,
                run,
                journal: Some(journal.with_fault(self.tasks.fault.clone())),
                halted: None,
                remembered: false,
            });
        }
        self.remember_tasks();
        if !problems.is_empty() {
            self.status = format!("wire: {}", problems.join(" · "));
        }
    }

    /// Keep the episode of every terminal task whose journal is complete.
    fn remember_tasks(&mut self) {
        let Some(home) = self.tasks.home.clone() else {
            return;
        };
        for live in &mut self.tasks.live {
            if live.remembered || live.halted.is_some() || live.run.announced().is_none() {
                continue;
            }
            let Some(episode) = Episode::derive(&live.run) else {
                continue;
            };
            match memory::remember(&home, &episode) {
                Ok(_) => live.remembered = true,
                Err(e) => self.status = format!("wire: memory: {}: {e}", live.run.id),
            }
        }
    }

    /// After every sync: hand new reports to running tasks, expire silent
    /// steps, send whatever became ready, and announce outcomes once. Each
    /// task's transitions are journaled before any of it is sent.
    pub fn advance_tasks(&mut self) {
        // Nothing is decided before the first sync: trust and membership
        // are read from it.
        if self.snapshot.is_none() {
            return;
        }
        let now_ms = now_ms();
        // Dispatch is authorized against the context as it is now, per room.
        let contexts: Vec<(String, Context)> = self
            .tasks
            .live
            .iter()
            .map(|l| (l.room.clone(), self.task_context(&l.room)))
            .collect();
        let rows = self.rows();
        let reports: Vec<(String, RoomMessage)> = rows
            .iter()
            .filter(|r| !r.outgoing())
            .filter_map(|r| {
                let m = r.room()?;
                (m.kind == Kind::Report).then(|| (r.peer().to_owned(), m))
            })
            .collect();
        // Room messages signed history holds as sent by this node.
        let synced = self.snapshot.is_some();
        let sent: HashSet<String> = rows
            .iter()
            .filter(|r| r.outgoing())
            .filter_map(|r| r.room().map(|m| m.id))
            .collect();
        drop(rows);
        let mut queue: Vec<(String, deaddrop_task::run::Message)> = Vec::new();
        let mut halted = Vec::new();
        let mut notes = Vec::new();
        for (live, (_, ctx)) in self.tasks.live.iter_mut().zip(&contexts) {
            if live.halted.is_some() {
                continue;
            }
            // Journaled as sent, missing from signed history: it never
            // left. The same message, the same id; never a new request.
            if live.resume && synced {
                live.resume = false;
                for m in live.run.messages() {
                    if !sent.contains(&m.id) {
                        queue.push((live.room.clone(), m));
                    }
                }
            }
            let before = live.run.journal().len();
            if live.run.outcome.is_none() {
                for (from, m) in &reports {
                    live.run.observe(from, m, now_ms);
                }
                live.run.expire(now_ms);
                live.run.ready(now_ms, ctx, || {
                    deaddrop_protocol::MessageId::generate().to_string()
                });
            }
            if live.run.outcome.is_some() && live.run.announced().is_none() {
                live.run
                    .announce(&deaddrop_protocol::MessageId::generate().to_string());
            }
            if live.run.journal().len() == before {
                continue;
            }
            if !commit(live) {
                let why = live.halted.clone().unwrap_or_default();
                halted.push(format!("{} halted · {why}", live.run.id));
                // Local only: sending anything would need the journal.
                notes.push(Note {
                    room: live.room.clone(),
                    lines: vec![
                        format!("task:: halted · {}", live.run.id),
                        "reason:: the task journal could not be written; nothing further was sent"
                            .into(),
                        "resume:: restart to continue from the last durable state".into(),
                    ],
                });
                continue;
            }
            // What this pass journaled to send: new requests, the outcome.
            let new_records = &live.run.journal()[before..];
            for m in live.run.messages() {
                let new = new_records.iter().any(|r| match r {
                    journal::Record::StepAssigned { request, .. } => *request == m.id,
                    journal::Record::Announced { message } => *message == m.id,
                    _ => false,
                });
                if new {
                    queue.push((live.room.clone(), m));
                }
            }
        }
        for (room, m) in queue {
            self.queue(&room, m);
        }
        self.tasks.notes.extend(notes);
        if !halted.is_empty() {
            self.status = format!("wire: {}", halted.join(" · "));
        }
        self.remember_tasks();
    }

    /// Tests only: make task journal appends fail after `fault` allows,
    /// for journals open now and opened later.
    #[doc(hidden)]
    pub fn inject_journal_fault(&mut self, fault: Fault) {
        for live in &mut self.tasks.live {
            if let Some(j) = live.journal.take() {
                live.journal = Some(j.with_fault(Some(fault.clone())));
            }
        }
        self.tasks.fault = Some(fault);
    }

    /// Whether a task in `room` was halted by a journal failure.
    pub fn task_halted(&self, room: &str) -> Option<&str> {
        self.tasks
            .live
            .iter()
            .filter(|l| l.room == room)
            .find_map(|l| l.halted.as_deref())
    }

    /// Queue one of a task's journaled room messages, under its own id.
    fn queue(&mut self, room: &str, m: deaddrop_task::run::Message) {
        if let Some(o) = self.room_outgoing(room, m.kind, m.mentions, m.text, Some(m.id)) {
            if m.wire {
                self.tasks.generated.insert(o.id.clone());
            }
            self.tasks.outbox.push(o);
        }
    }

    /// Whether `row` is the wire's (task lifecycle or step request) rather
    /// than yours: exactly the ids the task journals say the wire wrote,
    /// this session or any earlier one. Never read from the text: a line
    /// you type that looks like a task line is still yours.
    pub fn wire_authored(&self, row: &Row<'_>) -> bool {
        row.outgoing()
            && row
                .room()
                .is_some_and(|m| self.tasks.generated.contains(&m.id))
    }

    /// One room message from you, for every other member.
    fn room_outgoing(
        &self,
        room: &str,
        kind: Kind,
        mentions: Vec<String>,
        text: String,
        id: Option<String>,
    ) -> Option<RoomOutgoing> {
        let me = self.snapshot.as_ref().map(|s| s.node.id.clone());
        let config = self.rooms.iter().find(|r| r.name == room)?;
        let id = id.unwrap_or_else(|| deaddrop_protocol::MessageId::generate().to_string());
        let body = RoomMessage {
            room: room.to_owned(),
            id: id.clone(),
            kind,
            hop: 0,
            mentions,
            reply_to: None,
            status: None,
            text,
        }
        .encode();
        Some(RoomOutgoing {
            room: room.to_owned(),
            id,
            to: config
                .members
                .iter()
                .filter(|m| Some(*m) != me.as_ref())
                .cloned()
                .collect(),
            body,
            unknown: Vec::new(),
        })
    }

    /// Room messages tasks want sent, in order. Each goes out like any room
    /// message, then comes back to [`App::finish_task_send`].
    pub fn take_task_sends(&mut self) -> Vec<RoomOutgoing> {
        std::mem::take(&mut self.tasks.outbox)
    }

    /// How a task's room message went. Never touches the composer.
    pub fn finish_task_send(
        &mut self,
        outgoing: &RoomOutgoing,
        results: &[(String, Result<String, String>)],
    ) {
        if results.iter().any(|(_, r)| r.is_ok()) {
            self.room_sent_at
                .insert(outgoing.id.clone(), Instant::now());
            self.sent_at = Some(Instant::now());
            self.follow = true;
        }
        if let Some((member, Err(e))) = results.iter().find(|(_, r)| r.is_err()) {
            self.status = format!("wire: delivery to {} failed: {e}", short(member));
        }
    }

    pub fn task_planning(&self, room: &str) -> Option<&Planning> {
        self.tasks.planning.as_ref().filter(|p| p.room == room)
    }

    pub fn task_notes(&self, room: &str) -> impl Iterator<Item = &Note> {
        self.tasks.notes.iter().filter(move |n| n.room == room)
    }

    /// Every task's real trace in `room`, oldest first.
    pub fn task_traces(&self, room: &str) -> Vec<(&str, &[Trace])> {
        let mut out: Vec<(&str, &[Trace])> = self
            .tasks
            .logged
            .iter()
            .filter(|l| l.room == room)
            .map(|l| (l.id.as_str(), l.trace.as_slice()))
            .collect();
        out.extend(
            self.tasks
                .live
                .iter()
                .filter(|l| l.room == room)
                .map(|l| (l.run.id.as_str(), l.run.trace.as_slice())),
        );
        out
    }

    /// Whether any live task still has steps out.
    pub fn task_running(&self) -> bool {
        self.tasks
            .live
            .iter()
            .any(|l| l.run.outcome.is_none() && l.halted.is_none())
            || self.tasks.planning.is_some()
    }
}
