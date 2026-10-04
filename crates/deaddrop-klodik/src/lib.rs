//! Klodik V0: a conversational Deaddrop peer backed by a local model.
//!
//! One verified inbound message → one ACK (receipt) → one model call → one
//! signed reply, correlated to the inbound id. The adapter reads only what
//! `Shell::sync` has already verified and stored, and sends only through
//! `Shell::send`. The model ([`model`]) gets text and returns text.
//!
//! Restart safety: before any model call the adapter checks `Shell::sent`
//! for a reply already correlated to the message, and keeps a small state
//! file of what it has handled, including a model reply not yet sent — so a
//! failed send is retried without asking the model again.

pub mod context;
pub mod model;
pub mod timing;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use deaddrop_protocol::{CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId};
use deaddrop_room::{Kind, MAX_HOPS, RoomConfig, RoomMessage, Status, short};
use deaddrop_shell::{Relay, Shell, ShellError};
use serde::{Deserialize, Serialize};

use crate::model::{Model, ModelError, Prompt};

/// What answers a verified message: a model (Klodik) or a bounded worker
/// (another peer adapter). Gets the message, returns reply text or why
/// there is none. Never a stand-in reply.
pub trait Answer {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError>;

    /// Called once `reply` to `message` was sent, so a stateful answerer
    /// can record the completed exchange. Never called for a failure.
    fn replied(
        &mut self,
        _message: &EnvelopeV0,
        _reply: &str,
        _reply_id: &MessageId,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Answer a room request. `request` carries the request text as its
    /// body. By default, the same as a direct message.
    fn answer_room(&self, request: &EnvelopeV0, _room: &str) -> Result<String, ModelError> {
        self.answer(request)
    }

    /// Answer a room request, possibly with one structured request for
    /// another peer. By default: [`Answer::answer_room`], asking no one.
    fn answer_room_full(&self, request: &EnvelopeV0, room: &str) -> Result<RoomAnswer, ModelError> {
        Ok(RoomAnswer {
            text: self.answer_room(request, room)?,
            ask: None,
        })
    }

    /// A line said in a room this peer is in, by `author`, in the order it
    /// was handled here. Stateful answerers keep it as room context.
    fn observe_room(&mut self, _room: &str, _author: &str, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A room answer: the report text, and at most one structured request for
/// another peer. Prose never asks anyone; only `ask` can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomAnswer {
    pub text: String,
    pub ask: Option<Ask>,
}

/// One request this peer wants to put to another, in a room. `to` is a node
/// id or a member's short name. Sent only if this peer's policy allows it
/// and the hop budget has room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    pub to: String,
    pub text: String,
}

/// A model answers with the persona and the one message.
impl<M: Model> Answer for M {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        self.reply(&Prompt::new(message.from().as_str(), message.body()))
    }
}

/// Model failures before a message is given up on (and left unanswered).
pub const MAX_ATTEMPTS: u32 = 3;

/// What the adapter has done with one inbound message, by id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Handled {
    /// Answered; `reply` is the sent message id.
    Replied { reply: String },
    /// The model answered; the reply is not sent yet.
    PendingSend {
        text: String,
        /// A structured request to send after the reply, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ask: Option<Ask>,
    },
    /// The model failed this many times; retried until [`MAX_ATTEMPTS`].
    Failed { attempts: u32, error: String },
    /// From a trusted peer that may not talk to Klodik. Not answered.
    Refused,
    /// A room message not asking this peer: read, kept as context, not
    /// answered.
    Observed,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    messages: BTreeMap<String, Handled>,
}

/// Durable record of handled messages: one small JSON file, replaced
/// atomically, as the shell keeps its own files.
#[derive(Debug)]
pub struct State {
    path: PathBuf,
    file: StateFile,
}

impl State {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => StateFile::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            file,
        })
    }

    pub fn get(&self, id: &str) -> Option<&Handled> {
        self.file.messages.get(id)
    }

    fn set(&mut self, id: &str, handled: Handled) -> Result<(), String> {
        self.file.messages.insert(id.to_owned(), handled);
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(&self.file).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

/// What one step did, for logs and tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Step {
    /// Why the relay pass failed, if it did; local messages are still handled.
    pub sync_error: Option<String>,
    pub replied: Vec<String>,
    pub refused: Vec<String>,
    /// (message id, error) for anything left to retry or given up on.
    pub failed: Vec<(String, String)>,
    /// (message id, what) for room reports that reached some members but
    /// not others. The deliveries that worked stand.
    pub partial: Vec<(String, String)>,
    /// Messages this pass acted on in any way (answered, observed, refused,
    /// failed): a sign more may follow soon.
    pub handled: usize,
    /// (request id, to) for each request this peer put to another.
    pub asked: Vec<(String, String)>,
    /// (request id, why) for requests this peer wanted to put but may not.
    pub ask_refused: Vec<(String, String)>,
}

/// How long to wait before the next pass. There is no push from the relay,
/// so: right after a pass that handled something, look again soon; while
/// idle, back off step by step to `idle`. Never a busy loop.
#[derive(Debug, Clone)]
pub struct Pacer {
    pub active: std::time::Duration,
    pub idle: std::time::Duration,
    current: std::time::Duration,
}

impl Pacer {
    pub fn new(active: std::time::Duration, idle: std::time::Duration) -> Self {
        Self {
            active,
            idle: idle.max(active),
            current: active,
        }
    }

    pub fn next(&mut self, worked: bool) -> std::time::Duration {
        self.current = if worked {
            self.active
        } else {
            (self.current * 2).min(self.idle)
        };
        self.current
    }
}

/// An agent's ask, addressed the way a person addresses a peer. A
/// structured request (`request:: <capability>` first) names its target on
/// the line after; prose gets a leading mention.
pub fn addressed(target: &str, text: &str) -> String {
    match text.split_once('\n') {
        Some((head, rest)) if head.starts_with("request:: ") => {
            format!("{head}\n→ @{target}\n{rest}")
        }
        _ => format!("@{target} {text}"),
    }
}

/// Node ids from a comma-separated environment variable; empty if unset.
pub fn node_list(var: &str) -> Result<Vec<NodeId>, String> {
    match std::env::var(var).ok().filter(|v| !v.trim().is_empty()) {
        None => Ok(Vec::new()),
        Some(list) => list
            .split(',')
            .map(|id| NodeId::parse(id.trim()).map_err(|e| format!("{var} {id:?}: {e}")))
            .collect(),
    }
}

/// The idle poll from the environment: `<PREFIX>_POLL_MS`, else the older
/// `<PREFIX>_POLL_SECS`, else 500 ms.
pub fn idle_poll(prefix: &str) -> Result<std::time::Duration, String> {
    let get = |k: String| {
        std::env::var(&k)
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| (k, v))
    };
    if let Some((k, v)) = get(format!("{prefix}_POLL_MS")) {
        return v
            .parse()
            .map(std::time::Duration::from_millis)
            .map_err(|_| format!("{k} must be whole milliseconds, got {v:?}"));
    }
    if let Some((k, v)) = get(format!("{prefix}_POLL_SECS")) {
        return v
            .parse()
            .map(std::time::Duration::from_secs)
            .map_err(|_| format!("{k} must be whole seconds, got {v:?}"));
    }
    Ok(std::time::Duration::from_millis(500))
}

/// The same loop for any peer adapter, whatever answers behind it.
pub type Peer<R, A> = Klodik<R, A>;

pub struct Klodik<R, M> {
    pub shell: Shell<R>,
    pub model: M,
    pub state: State,
    /// Peers allowed to talk to Klodik. Already trusted; this narrows it.
    /// In a room, the peers whose requests this peer acts on.
    pub allowed: Vec<NodeId>,
    /// Rooms this peer is in. Membership grants nobody invocation.
    pub rooms: Vec<RoomConfig>,
    /// Peers this peer may put requests to (its own policy). Empty: none.
    pub may_ask: Vec<NodeId>,
}

impl<R: Relay, M: Answer> Klodik<R, M> {
    /// Sync, then handle every verified inbound message not yet answered.
    pub fn step(&mut self) -> Result<Step, ShellError> {
        let mut step = Step {
            sync_error: self.shell.sync().err().map(|e| e.to_string()),
            ..Step::default()
        };
        let sent = self.shell.sent()?;
        // `inbox` holds only messages `sync` verified against a trusted key.
        for message in self.shell.inbox()? {
            let id = message.id().to_string();
            let result = self.handle(&message, &sent, &mut step);
            if !matches!(result, Ok(None)) {
                step.handled += 1;
            }
            match result {
                Ok(Some(Handled::Replied { reply })) => step.replied.push(reply),
                Ok(Some(Handled::Refused)) => step.refused.push(id),
                Ok(_) => {}
                Err(error) => step.failed.push((id, error)),
            }
        }
        Ok(step)
    }

    /// `Ok(Some(_))` when this call newly settled the message.
    fn handle(
        &mut self,
        message: &EnvelopeV0,
        sent: &[EnvelopeV0],
        notes: &mut Step,
    ) -> Result<Option<Handled>, String> {
        let id = message.id().as_str();
        let pending = match self.state.get(id) {
            Some(Handled::Replied { .. } | Handled::Refused | Handled::Observed) => {
                return Ok(None);
            }
            Some(Handled::Failed { attempts, .. }) if *attempts >= MAX_ATTEMPTS => {
                return Ok(None);
            }
            Some(Handled::PendingSend { text, ask }) => Some((text.clone(), ask.clone())),
            Some(Handled::Failed { .. }) | None => None,
        };

        if let Some(room) = RoomMessage::decode(message.body()) {
            return self.handle_room(message, room, sent, pending, notes);
        }

        if !self.allowed.contains(message.from()) {
            self.state.set(id, Handled::Refused)?;
            return Ok(Some(Handled::Refused));
        }

        // Already answered (the state file may be older than the send).
        if let Some(reply) = sent.iter().find(|s| {
            s.to() == message.from() && s.correlation_id().map(|c| c.as_str()) == Some(id)
        }) {
            let done = Handled::Replied {
                reply: reply.id().to_string(),
            };
            self.state.set(id, done)?;
            return Ok(None);
        }

        timing::mark("T2", id);
        // Receipt only: Klodik has the message. Says nothing about a reply.
        self.shell
            .ack(message.id())
            .map_err(|e| format!("ack: {e}"))?;

        let text = match pending {
            Some((text, _)) => text,
            None => {
                timing::mark("T3", id);
                match self.model.answer(message) {
                    Ok(text) => {
                        timing::mark("T4", id);
                        self.state.set(
                            id,
                            Handled::PendingSend {
                                text: text.clone(),
                                ask: None,
                            },
                        )?;
                        text
                    }
                    Err(error) => return Err(self.model_failed(id, error)?),
                }
            }
        };

        let correlation = CorrelationId::parse(id).map_err(|e| format!("correlation: {e}"))?;
        let reply = self
            .shell
            .send(message.from(), &text, Some(correlation), &[])
            .map_err(|e| format!("send: {e}"))?;
        timing::mark("T5", id);
        let done = Handled::Replied {
            reply: reply.to_string(),
        };
        self.state.set(id, done.clone())?;
        self.model
            .replied(message, &text, &reply)
            .map_err(|e| format!("replied, but not recorded: {e}"))?;
        Ok(Some(done))
    }

    fn model_failed(&mut self, id: &str, error: ModelError) -> Result<String, String> {
        let attempts = match self.state.get(id) {
            Some(Handled::Failed { attempts, .. }) => attempts + 1,
            _ => 1,
        };
        let error = error.to_string();
        self.state.set(
            id,
            Handled::Failed {
                attempts,
                error: error.clone(),
            },
        )?;
        Ok(format!(
            "model (attempt {attempts}/{MAX_ATTEMPTS}): {error}"
        ))
    }

    /// A room message. Every member's message is acknowledged (receipt) and
    /// observed. Only a request that mentions this peer, from a sender this
    /// peer's policy allows, within the hop budget, is answered — with a
    /// report to the same room. A report never asks anyone anything.
    fn handle_room(
        &mut self,
        message: &EnvelopeV0,
        room: Result<RoomMessage, String>,
        sent: &[EnvelopeV0],
        pending: Option<(String, Option<Ask>)>,
        notes: &mut Step,
    ) -> Result<Option<Handled>, String> {
        let id = message.id().as_str();
        let me = self.shell.identity().node.to_string();
        let from = message.from().to_string();
        let config = room
            .as_ref()
            .ok()
            .and_then(|r| self.rooms.iter().find(|c| c.name == r.room))
            .cloned();
        let (Ok(request), Some(config)) = (room, config) else {
            // Malformed, or a room this peer is not in.
            self.state.set(id, Handled::Refused)?;
            return Ok(Some(Handled::Refused));
        };
        if !config.has(&from) || !config.has(&me) {
            self.state.set(id, Handled::Refused)?;
            return Ok(Some(Handled::Refused));
        }

        // Receipt only.
        self.shell
            .ack(message.id())
            .map_err(|e| format!("ack: {e}"))?;

        let asked = request.kind == Kind::Request && request.mentions.contains(&me);
        if !asked {
            self.model
                .observe_room(&config.name, &from, &request.text)?;
            self.state.set(id, Handled::Observed)?;
            return Ok(Some(Handled::Observed));
        }

        // Policy: trust gave the sender a channel; it may ask only if this
        // peer allows it, and only within the hop budget.
        let refusal = if !self.allowed.contains(message.from()) {
            Some(format!("{} may not ask {}", short(&from), short(&me)))
        } else if request.hop > MAX_HOPS {
            Some(format!("hop budget {MAX_HOPS} exceeded"))
        } else {
            None
        };
        if let Some(why) = refusal {
            self.report(&config, &request, message, Status::Refused, &why, notes)?;
            self.model
                .observe_room(&config.name, &from, &request.text)?;
            self.state.set(id, Handled::Refused)?;
            return Ok(Some(Handled::Refused));
        }

        // Already answered (the state file may be older than the send).
        if let Some(reply) = sent.iter().find(|s| {
            s.to() == message.from() && s.correlation_id().map(|c| c.as_str()) == Some(id)
        }) {
            let done = Handled::Replied {
                reply: reply.id().to_string(),
            };
            self.state.set(id, done)?;
            return Ok(None);
        }

        timing::mark_by("T2", &request.id, &me);
        let (text, ask) = match pending {
            Some(answer) => answer,
            None => {
                timing::mark_by("T3", &request.id, &me);
                let ask = EnvelopeV0::new(
                    message.id().clone(),
                    message.from().clone(),
                    message.to().clone(),
                    MessageKind::Message,
                    &request.text,
                );
                match self.model.answer_room_full(&ask, &config.name) {
                    Ok(answer) => {
                        timing::mark_by("T4", &request.id, &me);
                        self.state.set(
                            id,
                            Handled::PendingSend {
                                text: answer.text.clone(),
                                ask: answer.ask.clone(),
                            },
                        )?;
                        (answer.text, answer.ask)
                    }
                    Err(error) => {
                        let failure = self.model_failed(id, error)?;
                        // Given up: say so in the room, once, truthfully.
                        if matches!(self.state.get(id), Some(Handled::Failed { attempts, .. }) if *attempts >= MAX_ATTEMPTS)
                        {
                            let _ = self.report(
                                &config,
                                &request,
                                message,
                                Status::Failed,
                                "could not answer",
                                notes,
                            );
                        }
                        return Err(failure);
                    }
                }
            }
        };

        let reply = self.report(&config, &request, message, Status::Ok, &text, notes)?;
        timing::mark_by("T5", &request.id, &me);
        if let Some(ask) = ask {
            self.put(&config, &request, &ask, notes);
        }
        let done = Handled::Replied {
            reply: reply.to_string(),
        };
        self.state.set(id, done.clone())?;
        self.model
            .observe_room(&config.name, &from, &request.text)?;
        self.model.observe_room(&config.name, &me, &text)?;
        Ok(Some(done))
    }

    /// Put one structured request to another member, in the same room, as
    /// a request of the next hop that names what it was asked for. Only if
    /// this peer's policy allows asking that peer, the target is a room
    /// member, and the hop budget has room. Never more than one per answer.
    fn put(&mut self, config: &RoomConfig, request: &RoomMessage, ask: &Ask, notes: &mut Step) {
        let me = self.shell.identity().node.to_string();
        let target = config
            .members
            .iter()
            .find(|m| **m == ask.to || short(m) == ask.to)
            .cloned();
        let refusal = match &target {
            None => Some(format!("{} is not in #{}", ask.to, config.name)),
            Some(t) if *t == me => Some("a peer does not ask itself".into()),
            // A /task::wire step is the orchestrator's: the plan decides who
            // acts, so a worker on a task step creates no work of its own.
            Some(_) if request.text.starts_with("task:: ") => {
                Some("a task step asks no one: the plan decides".into())
            }
            Some(t) if !self.may_ask.iter().any(|m| m.as_str() == t) => {
                Some(format!("policy: {} may not ask {}", short(&me), short(t)))
            }
            Some(_) if request.hop >= MAX_HOPS => Some(format!("hop budget {MAX_HOPS} used")),
            Some(_) => None,
        };
        if let Some(why) = refusal {
            notes.ask_refused.push((request.id.clone(), why));
            return;
        }
        let target = target.expect("checked");
        let id = MessageId::generate().to_string();
        let body = RoomMessage {
            room: config.name.clone(),
            id: id.clone(),
            kind: Kind::Request,
            hop: request.hop + 1,
            mentions: vec![target.clone()],
            // What this request was made for: the request being answered.
            reply_to: Some(request.id.clone()),
            status: None,
            text: addressed(short(&target), &ask.text),
        }
        .encode();
        for member in &config.members {
            if *member == me {
                continue;
            }
            let sent = NodeId::parse(member.as_str())
                .map_err(|e| e.to_string())
                .and_then(|to| {
                    self.shell
                        .send(&to, &body, None, &[])
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = sent {
                notes
                    .partial
                    .push((id.clone(), format!("to {member}: {error}")));
            }
        }
        notes.asked.push((id, target));
    }

    /// Fan a report out to the room: the requester first, correlated to the
    /// request it answers (that delivery must work); then every other
    /// member. Deliveries that fail are noted; the ones that worked stand.
    fn report(
        &mut self,
        config: &RoomConfig,
        request: &RoomMessage,
        message: &EnvelopeV0,
        status: Status,
        text: &str,
        notes: &mut Step,
    ) -> Result<MessageId, String> {
        let me = self.shell.identity().node.to_string();
        let body = RoomMessage {
            room: config.name.clone(),
            id: MessageId::generate().to_string(),
            kind: Kind::Report,
            hop: request.hop,
            mentions: Vec::new(),
            reply_to: Some(request.id.clone()),
            status: Some(status),
            text: text.to_owned(),
        }
        .encode();
        let correlation =
            CorrelationId::parse(message.id().as_str()).map_err(|e| format!("correlation: {e}"))?;
        let reply = self
            .shell
            .send(message.from(), &body, Some(correlation), &[])
            .map_err(|e| format!("send: {e}"))?;
        for member in &config.members {
            if *member == me || member == message.from().as_str() {
                continue;
            }
            let delivered = NodeId::parse(member.as_str())
                .map_err(|e| e.to_string())
                .and_then(|to| {
                    self.shell
                        .send(&to, &body, None, &[])
                        .map_err(|e| e.to_string())
                });
            if let Err(error) = delivered {
                notes
                    .partial
                    .push((message.id().to_string(), format!("to {member}: {error}")));
            }
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod pacer_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn soon_after_work_backs_off_when_idle_never_spins() {
        let ms = Duration::from_millis;
        let mut p = Pacer::new(ms(100), ms(500));
        assert_eq!(p.next(true), ms(100));
        assert_eq!(p.next(false), ms(200));
        assert_eq!(p.next(false), ms(400));
        assert_eq!(p.next(false), ms(500));
        assert_eq!(p.next(false), ms(500), "capped");
        assert_eq!(p.next(true), ms(100), "work: look again soon");
        assert!(
            Pacer::new(ms(100), ms(0)).next(false) >= ms(100),
            "never below active"
        );
    }
}
