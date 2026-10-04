//! The Agent Bus V0 human path, headless: the TUI's own room composer and
//! stream over real `Shell`s, with three agent peers (fake runtimes).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use deaddrop_klodik::context::{Conversational, Transcripts};
use deaddrop_klodik::model::{Model, ModelError, Prompt};
use deaddrop_klodik::{Answer, Peer, State};
use deaddrop_protocol::{EnvelopeV0, NodeId};
use deaddrop_room::{Kind, RoomConfig};
use deaddrop_shell::{MemoryRelay, Shell, init};
use deaddrop_tui::app::{Action, App, Key, Row};
use deaddrop_tui::snapshot::load;

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const RESEARCH: &str = "research:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

#[derive(Clone, Default)]
struct Worker(Rc<RefCell<Vec<String>>>);

impl Answer for Worker {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        self.0.borrow_mut().push(message.body().to_owned());
        Ok(format!("[{}]", message.body()))
    }
}

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<String>>>);

impl Model for Recorder {
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError> {
        self.0.borrow_mut().push(prompt.user.clone());
        Ok("they found a release and checked a commit".into())
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
            .join("deaddrop-room-tui-tests")
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

    fn peer<A: Answer>(&self, who: &str, answer: A) -> Peer<&'r MemoryRelay, A> {
        let home = self.root.join(who);
        Peer {
            shell: Shell::open_with(&home, self.relay).unwrap(),
            model: answer,
            state: State::load(&home.join("peer/state.json")).unwrap(),
            allowed: vec![node(IVA)],
            rooms: vec![self.room.clone()],
            may_ask: Vec::new(),
        }
    }

    fn app(&self) -> App {
        let mut app = App::new();
        app.set_rooms(vec![self.room.clone()]);
        self.sync(&mut app);
        app.key(Key::ClickRoom(0));
        app
    }

    fn sync(&self, app: &mut App) {
        app.begin_refresh();
        app.finish(Ok(load(&self.iva).unwrap()));
    }

    /// Type and press Enter, then deliver as main does: one send each.
    /// Returns the status line the drop left.
    fn drop(&self, app: &mut App, text: &str) -> String {
        for c in text.chars() {
            app.key(Key::Char(c));
        }
        let Action::SendRoom(out) = app.key(Key::Enter) else {
            panic!("a room drop")
        };
        let results = out
            .to
            .iter()
            .map(|m| {
                let sent = self.iva.send(&node(m), &out.body, None, &[]);
                (
                    m.clone(),
                    sent.map(|id| id.to_string()).map_err(|e| e.to_string()),
                )
            })
            .collect();
        app.finish_room_send(&out, results);
        let status = app.status.clone();
        self.sync(app);
        status
    }
}

fn authors(app: &App) -> Vec<String> {
    app.current_thread()
        .iter()
        .map(|r| {
            if r.outgoing() {
                "you".into()
            } else {
                deaddrop_room::short(r.peer()).to_owned()
            }
        })
        .collect()
}

#[test]
fn research_github_then_klodik_answer_in_the_same_room() {
    let relay = MemoryRelay::default();
    let w = World::new("milestone", &relay);
    let (research_rt, github_rt, klodik_rt) =
        (Worker::default(), Worker::default(), Recorder::default());
    let mut research = w.peer(RESEARCH, research_rt.clone());
    let mut github = w.peer(GITHUB, github_rt.clone());
    let home = w.root.join(KLODIK);
    let mut klodik = w.peer(
        KLODIK,
        Conversational {
            model: klodik_rt.clone(),
            transcripts: Transcripts::load(&home.join("peer/transcripts.json")).unwrap(),
            own: KLODIK.into(),
        },
    );
    let mut app = w.app();
    let step_all = |r: &mut Peer<_, Worker>,
                    g: &mut Peer<_, Worker>,
                    k: &mut Peer<_, Conversational<Recorder>>| {
        r.step().unwrap();
        g.step().unwrap();
        k.step().unwrap();
    };

    let status = w.drop(&mut app, "@research find the latest official Rust release");
    assert!(
        status.contains("dropped to #d34ddr0p · relay took 3/3"),
        "{status}"
    );
    step_all(&mut research, &mut github, &mut klodik);
    w.drop(&mut app, "@github does commit 154f992 modify ui.rs?");
    step_all(&mut research, &mut github, &mut klodik);
    w.drop(&mut app, "@klodik summarize what they said");
    step_all(&mut research, &mut github, &mut klodik);
    w.sync(&mut app);

    // One stream, real authors, each logical message once.
    assert_eq!(
        authors(&app),
        ["you", "research", "you", "github", "you", "klodik"]
    );
    assert_eq!(research_rt.0.borrow().len(), 1);
    assert_eq!(github_rt.0.borrow().len(), 1);
    let prompt = klodik_rt.0.borrow().last().cloned().unwrap();
    assert!(
        prompt.contains("research:\n  [@research find the latest official Rust release]"),
        "{prompt}"
    );
    assert!(prompt.contains("github:\n  [@github does commit 154f992 modify ui.rs?]"));
    // Receipts are per delivery and say only that.
    let first = app.current_thread()[0].room().unwrap();
    assert_eq!(first.kind, Kind::Request);
    assert_eq!(app.room_receipts(&first.id), (3, 3));
    // The DM with Klodik holds none of this.
    let dm: Vec<Row<'_>> = app.thread(KLODIK);
    assert!(dm.is_empty(), "room messages stay in the room");
}

#[test]
fn plain_messages_and_unknown_mentions_ask_no_one() {
    let relay = MemoryRelay::default();
    let w = World::new("plain", &relay);
    let rt = Worker::default();
    let mut research = w.peer(RESEARCH, rt.clone());
    let mut app = w.app();
    w.drop(&mut app, "what do we all think?");
    let status = w.drop(&mut app, "@nobody are you there?");
    assert!(
        status.contains("@nobody not in room, not asked"),
        "{status}"
    );
    research.step().unwrap();
    assert!(rt.0.borrow().is_empty());
    assert_eq!(authors(&app), ["you", "you"]);
    assert!(
        app.current_thread()
            .iter()
            .all(|r| r.room().unwrap().kind == Kind::Message)
    );
}

#[test]
fn wire_commands_in_the_composer_are_intent_never_chat() {
    let relay = MemoryRelay::default();
    let w = World::new("wire", &relay);
    let mut app = w.app();
    for c in "Determine whether X helps.".chars() {
        app.key(Key::Char(c));
    }
    app.key(Key::Newline);
    for c in "/objective::wire --review".chars() {
        app.key(Key::Char(c));
    }
    assert_eq!(app.key(Key::Enter), Action::None, "not sent");
    assert!(app.status.contains("objective accepted as intent"));
    assert!(
        app.status.contains("no objective planner"),
        "honest gap: {}",
        app.status
    );
    assert!(app.composing(), "the draft stays");
    for _ in 0.."review".len() {
        app.key(Key::Backspace);
    }
    for c in "banana".chars() {
        app.key(Key::Char(c));
    }
    assert_eq!(app.key(Key::Enter), Action::None);
    assert_eq!(app.status, "wire: unknown flag '--banana'");
    w.iva.sync().unwrap();
    assert!(w.iva.sent().unwrap().is_empty(), "nothing left this node");
}
