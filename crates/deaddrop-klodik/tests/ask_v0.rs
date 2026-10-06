//! Bounded agent → agent requests: one structured ask, one hop, policy on
//! both ends, in the same room. Real `Shell`s, fake runtimes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use deaddrop_klodik::model::ModelError;
use deaddrop_klodik::{Answer, Ask, Peer, RoomAnswer, State};
use deaddrop_protocol::{EnvelopeV0, MessageId, NodeId};
use deaddrop_room::{Kind, RoomConfig, RoomMessage, Status};
use deaddrop_shell::{MemoryRelay, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const RESEARCH: &str = "research:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";
const QUESTION: &str = "does Tesliana-code/deaddrop@154f992cc3e88f51a0b6bdbf42998e94c3aeedaf modify crates/deaddrop-tui/src/ui.rs?";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

/// Research: always researches, and always wants to ask GitHub.
#[derive(Clone, Default)]
struct Research(Rc<RefCell<usize>>);

impl Answer for Research {
    fn answer(&self, _: &EnvelopeV0) -> Result<String, ModelError> {
        unreachable!("rooms use answer_room_full")
    }
    fn answer_room_full(&self, _: &EnvelopeV0, _: &str) -> Result<RoomAnswer, ModelError> {
        *self.0.borrow_mut() += 1;
        Ok(RoomAnswer {
            text: "found: the TUI layout landed in 154f992".into(),
            ask: Some(Ask {
                to: "github".into(),
                text: QUESTION.into(),
            }),
        })
    }
}

/// GitHub or Klodik: answers, records what it was asked.
#[derive(Clone, Default)]
struct Worker(Rc<RefCell<Vec<String>>>);

impl Answer for Worker {
    fn answer(&self, m: &EnvelopeV0) -> Result<String, ModelError> {
        self.0.borrow_mut().push(m.body().to_owned());
        Ok("yes, it modifies crates/deaddrop-tui/src/ui.rs".into())
    }
}

struct World<'r> {
    root: PathBuf,
    relay: &'r MemoryRelay,
    iva: Shell<&'r MemoryRelay>,
    room: RoomConfig,
}

impl<'r> World<'r> {
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-ask-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        let members = [IVA, KLODIK, GITHUB, RESEARCH];
        let shells: Vec<_> = members
            .map(|who| {
                let home = root.join(who);
                init(&home, node(who), RELAY).unwrap();
                Shell::open_with(&home, relay).unwrap()
            })
            .into();
        for a in &shells {
            for b in &shells {
                let card = b.identity();
                if card.node != a.identity().node {
                    a.add_peer(&card.node, &card.key).unwrap();
                }
            }
        }
        Self {
            iva: shells.into_iter().next().unwrap(),
            root,
            relay,
            room: RoomConfig {
                name: "d34ddr0p".into(),
                members: members.map(str::to_owned).to_vec(),
            },
        }
    }

    fn peer<A: Answer>(
        &self,
        who: &str,
        answer: A,
        allowed: &[&str],
        may_ask: &[&str],
    ) -> Peer<&'r MemoryRelay, A> {
        let home = self.root.join(who);
        Peer {
            shell: Shell::open_with(&home, self.relay).unwrap(),
            model: answer,
            state: State::load(&home.join("peer/state.json")).unwrap(),
            allowed: allowed.iter().map(|a| node(a)).collect(),
            rooms: vec![self.room.clone()],
            may_ask: may_ask.iter().map(|a| node(a)).collect(),
        }
    }

    fn say(&self, from: &Shell<&'r MemoryRelay>, hop: u8, to: &str, text: &str) -> String {
        let id = MessageId::generate().to_string();
        let body = RoomMessage {
            room: "d34ddr0p".into(),
            id: id.clone(),
            kind: Kind::Request,
            hop,
            mentions: vec![to.into()],
            reply_to: None,
            status: None,
            text: text.into(),
        }
        .encode();
        let me = from.identity().node.to_string();
        for m in &self.room.members {
            if *m != me {
                from.send(&node(m), &body, None, &[]).unwrap();
            }
        }
        id
    }

    /// The room as Iva holds it: (author, message).
    fn room(&self) -> Vec<(String, RoomMessage)> {
        self.iva.sync().unwrap();
        self.iva
            .inbox()
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
fn research_asks_github_once_and_the_chain_stops() {
    let relay = MemoryRelay::default();
    let w = World::new("chain", &relay);
    let research_rt = Research::default();
    let (github_rt, klodik_rt) = (Worker::default(), Worker::default());
    let mut research = w.peer(RESEARCH, research_rt.clone(), &[IVA], &[GITHUB]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA, RESEARCH], &[]);
    let mut klodik = w.peer(KLODIK, klodik_rt.clone(), &[IVA], &[]);

    let asked = w.say(
        &w.iva,
        0,
        RESEARCH,
        "@research investigate X and ask GitHub whether the repository contains it",
    );
    let step = research.step().unwrap();
    assert_eq!(step.asked.len(), 1, "{step:?}");
    // Many rounds for everyone: exactly one research run, one github run.
    for _ in 0..5 {
        research.step().unwrap();
        github.step().unwrap();
        klodik.step().unwrap();
    }
    assert_eq!(*research_rt.0.borrow(), 1);
    assert_eq!(
        github_rt.0.borrow().as_slice(),
        [format!("@github {QUESTION}")]
    );
    assert!(klodik_rt.0.borrow().is_empty(), "nobody asked klodik");

    let room = w.room();
    let by = |who: &str, kind: Kind| {
        room.iter()
            .filter(|(f, m)| f == who && m.kind == kind)
            .cloned()
            .collect::<Vec<_>>()
    };
    let [(_, research_report)] = by(RESEARCH, Kind::Report).try_into().unwrap();
    assert_eq!(research_report.reply_to.as_deref(), Some(asked.as_str()));
    let [(_, ask)] = by(RESEARCH, Kind::Request).try_into().unwrap();
    assert_eq!(ask.mentions, [GITHUB]);
    assert_eq!(ask.hop, 1, "one hop");
    assert_eq!(
        ask.reply_to.as_deref(),
        Some(asked.as_str()),
        "made for Iva's request"
    );
    assert_eq!(ask.text, format!("@github {QUESTION}"));
    let [(_, github_report)] = by(GITHUB, Kind::Report).try_into().unwrap();
    assert_eq!(github_report.reply_to.as_deref(), Some(ask.id.as_str()));
    assert_eq!(github_report.status, Some(Status::Ok));
    assert_eq!(github_report.room, "d34ddr0p", "the same room");
    assert!(github_report.mentions.is_empty(), "a report asks no one");
}

#[test]
fn without_policy_research_asks_no_one() {
    let relay = MemoryRelay::default();
    let w = World::new("no-policy", &relay);
    let github_rt = Worker::default();
    let mut research = w.peer(RESEARCH, Research::default(), &[IVA], &[]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA, RESEARCH], &[]);
    w.say(
        &w.iva,
        0,
        RESEARCH,
        "@research investigate X and ask GitHub",
    );
    let step = research.step().unwrap();
    assert!(step.asked.is_empty());
    assert_eq!(step.ask_refused[0].1, "policy: research may not ask github");
    github.step().unwrap();
    assert!(github_rt.0.borrow().is_empty());
}

#[test]
fn github_refuses_when_its_own_policy_does_not_allow_research() {
    let relay = MemoryRelay::default();
    let w = World::new("github-says-no", &relay);
    let github_rt = Worker::default();
    let mut research = w.peer(RESEARCH, Research::default(), &[IVA], &[GITHUB]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA], &[]);
    w.say(
        &w.iva,
        0,
        RESEARCH,
        "@research investigate X and ask GitHub",
    );
    research.step().unwrap();
    assert_eq!(github.step().unwrap().refused.len(), 1);
    assert!(github_rt.0.borrow().is_empty(), "no worker ran");
    let refused = w.room().into_iter().any(|(f, m)| {
        f == GITHUB && m.status == Some(Status::Refused) && m.text == "research may not ask github"
    });
    assert!(refused, "visible in the room");
}

#[test]
fn an_agent_asked_by_an_agent_cannot_ask_further() {
    let relay = MemoryRelay::default();
    let w = World::new("hop", &relay);
    let klodik = w.peer(KLODIK, Worker::default(), &[IVA], &[]);
    let github_rt = Worker::default();
    // Research acts on Klodik's request (hop 1), but its own ask would be hop 2.
    let mut research = w.peer(RESEARCH, Research::default(), &[IVA, KLODIK], &[GITHUB]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA, RESEARCH], &[]);
    w.say(&klodik.shell, 1, RESEARCH, "@research look and ask GitHub");
    let step = research.step().unwrap();
    assert_eq!(step.replied.len(), 1);
    assert!(step.asked.is_empty());
    assert_eq!(step.ask_refused[0].1, "hop budget 1 used");
    github.step().unwrap();
    assert!(github_rt.0.borrow().is_empty());
}

#[test]
fn a_restart_does_not_ask_twice() {
    let relay = MemoryRelay::default();
    let w = World::new("restart", &relay);
    let rt = Research::default();
    w.say(
        &w.iva,
        0,
        RESEARCH,
        "@research investigate X and ask GitHub",
    );
    w.peer(RESEARCH, rt.clone(), &[IVA], &[GITHUB])
        .step()
        .unwrap();
    let again = w
        .peer(RESEARCH, rt.clone(), &[IVA], &[GITHUB])
        .step()
        .unwrap();
    assert!(again.asked.is_empty());
    assert_eq!(*rt.0.borrow(), 1);
    let asks = w
        .room()
        .into_iter()
        .filter(|(_, m)| m.kind == Kind::Request)
        .count();
    assert_eq!(asks, 1);
}

#[test]
fn a_structured_ask_names_its_target_on_its_own_line() {
    let text = "request:: github.inspect\nrepo:: o/r\ncommit:: abc\npath:: x";
    assert_eq!(
        deaddrop_klodik::addressed("github", text),
        "request:: github.inspect\n→ @github\nrepo:: o/r\ncommit:: abc\npath:: x"
    );
    assert_eq!(
        deaddrop_klodik::addressed("github", "does it?"),
        "@github does it?"
    );
}

#[test]
fn a_task_step_creates_no_work_of_its_own() {
    let relay = MemoryRelay::default();
    let w = World::new("task-step", &relay);
    let github_rt = Worker::default();
    let mut research = w.peer(RESEARCH, Research::default(), &[IVA], &[GITHUB]);
    let mut github = w.peer(GITHUB, github_rt.clone(), &[IVA, RESEARCH], &[]);
    w.say(
        &w.iva,
        0,
        RESEARCH,
        "task:: T-1 · research_release\nfind X and ask GitHub",
    );
    let step = research.step().unwrap();
    assert_eq!(step.replied.len(), 1, "the step itself is answered");
    assert!(step.asked.is_empty());
    assert_eq!(
        step.ask_refused[0].1,
        "a task step asks no one: the plan decides"
    );
    github.step().unwrap();
    assert!(github_rt.0.borrow().is_empty());
}
