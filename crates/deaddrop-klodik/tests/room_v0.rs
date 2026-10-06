//! Agent Bus V0: a shared room over ordinary signed messages, on real
//! `Shell`s and an in-process relay. Runtimes are fakes that count calls.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use deaddrop_klodik::context::{Conversational, Transcripts};
use deaddrop_klodik::model::{Model, ModelError, Prompt};
use deaddrop_klodik::{Answer, Handled, MAX_ATTEMPTS, Peer, State};
use deaddrop_protocol::{EnvelopeV0, MessageId, NodeId};
use deaddrop_room::{Kind, RoomConfig, RoomMessage, Status};
use deaddrop_shell::{MemoryRelay, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const RESEARCH: &str = "research:agent:deaddrop";
const DANIL: &str = "danil:local:deaddrop";
const GHOST: &str = "ghost:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";
const ROOM: &str = "d34ddr0p";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

/// A runtime that records what it was asked and answers from a script.
#[derive(Clone, Default)]
struct Worker {
    asked: Rc<RefCell<Vec<String>>>,
    fail: Rc<Cell<bool>>,
}

impl Answer for Worker {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        self.asked.borrow_mut().push(message.body().to_owned());
        if self.fail.get() {
            Err(ModelError::Empty)
        } else {
            Ok(format!("answer to: {}", message.body()))
        }
    }
}

/// Klodik's model: records prompts.
#[derive(Clone, Default)]
struct Recorder {
    prompts: Rc<RefCell<Vec<String>>>,
}

impl Model for Recorder {
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError> {
        self.prompts.borrow_mut().push(prompt.user.clone());
        Ok("summary: research found a release, github checked the commit".into())
    }
}

struct World<'r> {
    root: PathBuf,
    relay: &'r MemoryRelay,
    iva: Shell<&'r MemoryRelay>,
    danil: Shell<&'r MemoryRelay>,
    room: RoomConfig,
}

impl<'r> World<'r> {
    /// Iva and three agents share a room and all trust each other. Danil
    /// is trusted by everyone but is not in the room. Ghost is listed as a
    /// member but nobody trusts it, so deliveries to it fail.
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-room-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        let open = |who: &str| {
            let home = root.join(who);
            init(&home, node(who), RELAY).unwrap();
            Shell::open_with(&home, relay).unwrap()
        };
        let shells: Vec<_> = [IVA, KLODIK, GITHUB, RESEARCH, DANIL].map(open).into();
        for a in &shells {
            for b in &shells {
                let card = b.identity();
                if card.node != a.identity().node {
                    a.add_peer(&card.node, &card.key).unwrap();
                }
            }
        }
        let mut shells = shells.into_iter();
        let iva = shells.next().unwrap();
        let danil = shells.nth(3).unwrap();
        Self {
            root,
            relay,
            iva,
            danil,
            room: RoomConfig {
                name: ROOM.into(),
                members: [IVA, KLODIK, GITHUB, RESEARCH, GHOST]
                    .map(str::to_owned)
                    .to_vec(),
            },
        }
    }

    fn peer<A: Answer>(&self, who: &str, answer: A, allowed: &[&str]) -> Peer<&'r MemoryRelay, A> {
        let home = self.root.join(who);
        Peer {
            shell: Shell::open_with(&home, self.relay).unwrap(),
            model: answer,
            state: State::load(&home.join("peer/state.json")).unwrap(),
            allowed: allowed.iter().map(|a| node(a)).collect(),
            rooms: vec![self.room.clone()],
            may_ask: Vec::new(),
        }
    }

    fn klodik(&self, model: Recorder) -> Peer<&'r MemoryRelay, Conversational<Recorder>> {
        let home = self.root.join(KLODIK);
        self.peer(
            KLODIK,
            Conversational {
                model,
                transcripts: Transcripts::load(&home.join("peer/transcripts.json")).unwrap(),
                own: KLODIK.into(),
            },
            &[IVA],
        )
    }

    /// Fan a room message out from `from` to every other member it trusts.
    fn say(
        &self,
        from: &Shell<&'r MemoryRelay>,
        kind: Kind,
        hop: u8,
        mentions: &[&str],
        text: &str,
    ) -> String {
        let id = MessageId::generate().to_string();
        let body = RoomMessage {
            room: ROOM.into(),
            id: id.clone(),
            kind,
            hop,
            mentions: mentions.iter().map(|m| (*m).to_owned()).collect(),
            reply_to: None,
            status: None,
            text: text.into(),
        }
        .encode();
        let me = from.identity().node.to_string();
        for member in &self.room.members {
            if *member != me {
                let _ = from.send(&node(member), &body, None, &[]);
            }
        }
        id
    }

    /// Room messages `who` holds, decoded, with their verified sender.
    fn room_inbox(&self, who: &Shell<&'r MemoryRelay>) -> Vec<(String, RoomMessage)> {
        who.sync().unwrap();
        who.inbox()
            .unwrap()
            .iter()
            .filter_map(|e| {
                RoomMessage::decode(e.body())
                    .and_then(Result::ok)
                    .map(|m| (e.from().to_string(), m))
            })
            .collect()
    }
}

#[test]
fn a_mention_invokes_only_that_agent_and_the_report_returns_to_the_room() {
    let relay = MemoryRelay::default();
    let w = World::new("mention", &relay);
    let (research_rt, github_rt) = (Worker::default(), Worker::default());
    let mut research = w.peer(RESEARCH, research_rt.clone(), &[IVA]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA]);
    let mut klodik = w.klodik(Recorder::default());

    let asked = w.say(
        &w.iva,
        Kind::Request,
        0,
        &[RESEARCH],
        "@research find the latest Rust release",
    );
    let step = research.step().unwrap();
    assert_eq!(step.replied.len(), 1);
    assert_eq!(
        step.partial.len(),
        1,
        "ghost is untrusted: noted, not fatal"
    );
    github.step().unwrap();
    klodik.step().unwrap();
    assert_eq!(research_rt.asked.borrow().len(), 1);
    assert!(
        github_rt.asked.borrow().is_empty(),
        "not mentioned, not invoked"
    );
    assert!(klodik.model.model.prompts.borrow().is_empty());

    // Iva gets the report in the same room, from Research, answering her.
    let reports: Vec<_> = w
        .room_inbox(&w.iva)
        .into_iter()
        .filter(|(_, m)| m.kind == Kind::Report)
        .collect();
    let [(from, report)] = reports.as_slice() else {
        panic!("{reports:?}")
    };
    assert_eq!(from, RESEARCH);
    assert_eq!(report.room, ROOM);
    assert_eq!(report.reply_to.as_deref(), Some(asked.as_str()));
    assert_eq!(report.status, Some(Status::Ok));
    assert!(report.mentions.is_empty(), "a report asks nobody");
    // The other agents got the report too: it is the room's.
    assert!(
        w.room_inbox(&github.shell)
            .iter()
            .any(|(f, m)| f == RESEARCH && m.kind == Kind::Report)
    );
}

#[test]
fn a_plain_room_message_invokes_nobody() {
    let relay = MemoryRelay::default();
    let w = World::new("plain", &relay);
    let rts = [Worker::default(), Worker::default()];
    let mut research = w.peer(RESEARCH, rts[0].clone(), &[IVA]);
    let mut github = w.peer(GITHUB, rts[1].clone(), &[IVA]);
    w.say(
        &w.iva,
        Kind::Message,
        0,
        &[],
        "what do we think about this?",
    );
    let r = research.step().unwrap();
    github.step().unwrap();
    assert!(r.replied.is_empty());
    assert!(rts.iter().all(|rt| rt.asked.borrow().is_empty()));
    assert!(matches!(
        research
            .state
            .get(research.shell.inbox().unwrap()[0].id().as_str()),
        Some(Handled::Observed)
    ));
    // Every physical delivery was acknowledged: receipt only.
    w.iva.sync().unwrap();
    let acked =
        w.iva
            .sent()
            .unwrap()
            .iter()
            .filter(|e| {
                w.iva.delivery(e.id()).unwrap().iter().any(|d| {
                    d.kind() == deaddrop_protocol::DeliveryEventKind::RecipientAcknowledged
                })
            })
            .count();
    assert_eq!(
        acked, 2,
        "research and github acknowledged; klodik and ghost did not run"
    );
}

#[test]
fn klodik_summarizes_from_the_room_he_observed() {
    let relay = MemoryRelay::default();
    let w = World::new("summary", &relay);
    let mut research = w.peer(RESEARCH, Worker::default(), &[IVA]);
    let mut github = w.peer(GITHUB, Worker::default(), &[IVA]);
    let recorder = Recorder::default();
    let mut klodik = w.klodik(recorder.clone());
    w.say(
        &w.iva,
        Kind::Request,
        0,
        &[RESEARCH],
        "@research latest Rust release?",
    );
    research.step().unwrap();
    w.say(
        &w.iva,
        Kind::Request,
        0,
        &[GITHUB],
        "@github does commit X modify Y?",
    );
    github.step().unwrap();
    klodik.step().unwrap(); // observes both requests and both reports
    w.say(
        &w.iva,
        Kind::Request,
        0,
        &[KLODIK],
        "@klodik summarize what they said",
    );
    klodik.step().unwrap();
    let prompt = recorder.prompts.borrow().last().cloned().unwrap();
    assert!(prompt.starts_with("ROOM #d34ddr0p SO FAR"), "{prompt}");
    for said in [
        "research:\n  answer to: @research latest Rust release?",
        "github:\n  answer to: @github does commit X modify Y?",
    ] {
        assert!(prompt.contains(said), "{said:?} in {prompt}");
    }
    assert!(
        prompt.ends_with("REQUEST FROM iva IN #d34ddr0p\n\n  @klodik summarize what they said")
    );
}

#[test]
fn agent_to_agent_requests_need_policy_and_respect_the_hop_budget() {
    let relay = MemoryRelay::default();
    let w = World::new("agents", &relay);
    let github_rt = Worker::default();
    // GitHub's operator allows Iva and Research to ask it; not Klodik.
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA, RESEARCH]);
    let research = w.peer(RESEARCH, Worker::default(), &[IVA]);
    let klodik = w.peer(KLODIK, Worker::default(), &[IVA]);

    // Allowed, one hop: works.
    w.say(
        &research.shell,
        Kind::Request,
        1,
        &[GITHUB],
        "@github does commit X modify Y?",
    );
    assert_eq!(github.step().unwrap().replied.len(), 1);
    assert_eq!(github_rt.asked.borrow().len(), 1);

    // Trusted and in the room, but not allowed by GitHub: refused, visibly.
    w.say(
        &klodik.shell,
        Kind::Request,
        1,
        &[GITHUB],
        "@github inspect please",
    );
    assert_eq!(github.step().unwrap().refused.len(), 1);
    // A second hop is past the budget, even from an allowed sender.
    w.say(
        &research.shell,
        Kind::Request,
        2,
        &[GITHUB],
        "@github again",
    );
    assert_eq!(github.step().unwrap().refused.len(), 1);
    assert_eq!(
        github_rt.asked.borrow().len(),
        1,
        "no worker ran for either"
    );

    let reports: Vec<_> = w
        .room_inbox(&w.iva)
        .into_iter()
        .filter(|(f, m)| f == GITHUB && m.kind == Kind::Report)
        .map(|(_, m)| (m.status, m.text))
        .collect();
    assert!(reports.contains(&(Some(Status::Refused), "klodik may not ask github".into())));
    assert!(reports.contains(&(Some(Status::Refused), "hop budget 1 exceeded".into())));
}

#[test]
fn reports_never_trigger_anyone_so_there_is_no_loop() {
    let relay = MemoryRelay::default();
    let w = World::new("no-loop", &relay);
    let rts = [Worker::default(), Worker::default(), Worker::default()];
    let mut peers = [
        w.peer(RESEARCH, rts[0].clone(), &[IVA, GITHUB, KLODIK]),
        w.peer(GITHUB, rts[1].clone(), &[IVA, RESEARCH, KLODIK]),
        w.peer(KLODIK, rts[2].clone(), &[IVA, RESEARCH, GITHUB]),
    ];
    w.say(&w.iva, Kind::Request, 0, &[RESEARCH], "@research go");
    // Let everyone run many rounds: exactly one invocation ever happens.
    for _ in 0..5 {
        for p in &mut peers {
            p.step().unwrap();
        }
    }
    let total: usize = rts.iter().map(|rt| rt.asked.borrow().len()).sum();
    assert_eq!(total, 1);
}

#[test]
fn membership_is_enforced_and_unknown_rooms_are_ignored() {
    let relay = MemoryRelay::default();
    let w = World::new("membership", &relay);
    let rt = Worker::default();
    let mut research = w.peer(RESEARCH, rt.clone(), &[IVA, DANIL]);
    // Danil is trusted and even allowed, but not in the room.
    let body = RoomMessage {
        room: ROOM.into(),
        id: "x".into(),
        kind: Kind::Request,
        hop: 0,
        mentions: vec![RESEARCH.into()],
        reply_to: None,
        status: None,
        text: "@research hi".into(),
    }
    .encode();
    w.danil.send(&node(RESEARCH), &body, None, &[]).unwrap();
    let other_room = body.replace("room: d34ddr0p", "room: elsewhere");
    w.iva.send(&node(RESEARCH), &other_room, None, &[]).unwrap();
    assert_eq!(research.step().unwrap().refused.len(), 2);
    assert!(rt.asked.borrow().is_empty());
}

#[test]
fn a_failing_runtime_reports_failure_once_and_restart_does_not_repeat() {
    let relay = MemoryRelay::default();
    let w = World::new("failure", &relay);
    let rt = Worker::default();
    rt.fail.set(true);
    let mut research = w.peer(RESEARCH, rt.clone(), &[IVA]);
    w.say(
        &w.iva,
        Kind::Request,
        0,
        &[RESEARCH],
        "@research impossible",
    );
    for _ in 0..(MAX_ATTEMPTS + 2) {
        let _ = research.step();
    }
    assert_eq!(rt.asked.borrow().len(), MAX_ATTEMPTS as usize);
    let failed: Vec<_> = w
        .room_inbox(&w.iva)
        .into_iter()
        .filter(|(_, m)| m.status == Some(Status::Failed))
        .collect();
    assert_eq!(failed.len(), 1, "said once, truthfully");

    // A healthy request, then a restart: no second report.
    rt.fail.set(false);
    w.say(&w.iva, Kind::Request, 0, &[RESEARCH], "@research fine");
    research.step().unwrap();
    let again = rt.clone();
    let mut restarted = w.peer(RESEARCH, again, &[IVA]);
    restarted.step().unwrap();
    let oks = w
        .room_inbox(&w.iva)
        .into_iter()
        .filter(|(_, m)| m.status == Some(Status::Ok))
        .count();
    assert_eq!(oks, 1);
}
