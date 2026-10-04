//! `/task::wire` in a room: the TUI side of `deaddrop-task`.
//!
//! The parser decides a draft is a task; the planner (off the UI thread)
//! proposes; `deaddrop_task::plan::validate` decides; a live task runs as a
//! `deaddrop_task::run::TaskRun`, whose Agent Wire history decides each
//! step and the task. This module only carries messages: it turns
//! dispatches into ordinary room requests, feeds room reports back in, and
//! says what happened. A dry run sends nothing at all.

use std::collections::HashSet;
use std::time::Instant;

use deaddrop_room::{Kind, RoomMessage, short};
use deaddrop_task::plan::{Plan, Proposed};
use deaddrop_task::registry::Context;
use deaddrop_task::run::{Dispatch, Outcome, TaskRun, Trace};

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

#[derive(Debug, Clone)]
pub struct Live {
    pub room: String,
    pub run: TaskRun,
    announced: bool,
}

#[derive(Debug, Clone)]
pub struct Tasks {
    pub planning: Option<Planning>,
    pub live: Vec<Live>,
    pub notes: Vec<Note>,
    pub logged: Vec<Logged>,
    outbox: Vec<RoomOutgoing>,
    /// Room reports already handed to a run.
    seen: HashSet<String>,
    /// Room message ids the task layer wrote this session: lifecycle
    /// events and step requests. Shown as `λ wire`, never as `you`.
    generated: HashSet<String>,
    epoch: Instant,
}

impl Default for Tasks {
    fn default() -> Self {
        Self {
            planning: None,
            live: Vec::new(),
            notes: Vec::new(),
            logged: Vec::new(),
            outbox: Vec::new(),
            seen: HashSet::new(),
            generated: HashSet::new(),
            epoch: Instant::now(),
        }
    }
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
        // What you wrote, as you wrote it: yours.
        let yours = format!("{}\n\n/task::wire", payload.trim());
        if let Some(o) = self.room_outgoing(&room, Kind::Message, Vec::new(), yours, None) {
            self.tasks.outbox.push(o);
        }
        // That it was accepted: the wire's, not yours.
        let text = format!("task:: accepted\nid:: {id}\nsteps:: {steps}");
        self.task_message(&room, Kind::Message, Vec::new(), text, None);
        self.tasks.live.push(Live {
            room,
            run: TaskRun::new(id, plan),
            announced: false,
        });
        self.status = format!("wire: task {id} accepted · {steps} steps");
        self.advance_tasks();
    }

    /// After every sync: hand new reports to running tasks, expire silent
    /// steps, send whatever became ready, and announce outcomes once.
    pub fn advance_tasks(&mut self) {
        let now_ms = self.tasks.epoch.elapsed().as_millis() as u64;
        let reports: Vec<(String, RoomMessage)> = self
            .rows()
            .iter()
            .filter(|r| !r.outgoing())
            .filter_map(|r| {
                let m = r.room()?;
                (m.kind == Kind::Report).then(|| (r.peer().to_owned(), m))
            })
            .collect();
        let mut sends: Vec<(String, Dispatch)> = Vec::new();
        let mut done: Vec<(String, String)> = Vec::new();
        for live in &mut self.tasks.live {
            if live.run.outcome.is_none() {
                for (from, m) in &reports {
                    if !self.tasks.seen.contains(&m.id) && live.run.observe(from, m) {
                        self.tasks.seen.insert(m.id.clone());
                    }
                }
                live.run.expire(now_ms);
                let ready = live.run.ready(now_ms, || {
                    deaddrop_protocol::MessageId::generate().to_string()
                });
                sends.extend(ready.into_iter().map(|d| (live.room.clone(), d)));
            }
            if let (Some(outcome), false) = (&live.run.outcome, live.announced) {
                live.announced = true;
                let id = &live.run.id;
                let text = match outcome {
                    Outcome::Complete => {
                        let by = live
                            .run
                            .result()
                            .map(|r| short(&r.from).to_owned())
                            .unwrap_or_default();
                        format!("task:: complete\nid:: {id}\nresult:: the {by} report above")
                    }
                    Outcome::Failed(why) => format!("task:: failed\nid:: {id}\nreason:: {why}"),
                };
                done.push((live.room.clone(), text));
            }
        }
        for (room, d) in sends {
            let mention = vec![d.worker.clone()];
            self.task_message(&room, Kind::Request, mention, d.text, Some(d.request));
        }
        for (room, text) in done {
            self.task_message(&room, Kind::Message, Vec::new(), text, None);
        }
    }

    /// A room message the task layer writes: queued, and remembered as the
    /// wire's own so it is never shown as yours.
    fn task_message(
        &mut self,
        room: &str,
        kind: Kind,
        mentions: Vec<String>,
        text: String,
        id: Option<String>,
    ) {
        if let Some(o) = self.room_outgoing(room, kind, mentions, text, id) {
            self.tasks.generated.insert(o.id.clone());
            self.tasks.outbox.push(o);
        }
    }

    /// Whether `row` is the wire's (task lifecycle or step request) rather
    /// than yours. This session: exactly the ids the task layer wrote. From
    /// an earlier session, where those ids are gone: your room messages
    /// that are task lines (`task:: …` first), which only the wire writes.
    pub fn wire_authored(&self, row: &Row<'_>) -> bool {
        if !row.outgoing() {
            return false;
        }
        let Some(m) = row.room() else {
            return false;
        };
        self.tasks.generated.contains(&m.id)
            || (self.is_earlier(row.id()) && m.text.starts_with("task:: "))
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
        self.tasks.live.iter().any(|l| l.run.outcome.is_none()) || self.tasks.planning.is_some()
    }
}
