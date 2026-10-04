//! The product loop, headless: Iva types in the TUI composer, the TUI sends
//! through its normal `Shell::send` path, Klodik answers (fake model), and
//! the TUI's ordinary auto-sync shows the reply in the same conversation.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Instant;

use deaddrop_klodik::model::{Model, ModelError, Prompt};
use deaddrop_klodik::{Klodik, State};
use deaddrop_protocol::NodeId;
use deaddrop_shell::{MemoryRelay, Shell, init};
use deaddrop_tui::app::{Action, App, Key, Row};
use deaddrop_tui::snapshot::{load, send};

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

struct Fake {
    calls: Cell<usize>,
    answer: Result<&'static str, ModelError>,
}

impl Model for Fake {
    fn reply(&self, _: &Prompt) -> Result<String, ModelError> {
        self.calls.set(self.calls.get() + 1);
        self.answer.clone().map(str::to_owned)
    }
}

fn setup<'r>(
    test: &str,
    relay: &'r MemoryRelay,
) -> (Shell<&'r MemoryRelay>, Shell<&'r MemoryRelay>, PathBuf) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-klodik-tui-tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&root);
    let open = |who: &str| {
        let home = root.join(who);
        init(&home, NodeId::parse(who).unwrap(), RELAY).unwrap();
        Shell::open_with(&home, relay).unwrap()
    };
    let (iva, klodik) = (open(IVA), open(KLODIK));
    let (i, k) = (iva.identity(), klodik.identity());
    iva.add_peer(&k.node, &k.key).unwrap();
    klodik.add_peer(&i.node, &i.key).unwrap();
    (
        iva,
        klodik,
        root.join(KLODIK).join("klodik").join("state.json"),
    )
}

/// Type `text` into the open conversation and send it the way the TUI does.
fn type_and_send(app: &mut App, iva: &Shell<&MemoryRelay>, text: &str) -> String {
    for c in text.chars() {
        app.key(Key::Char(c));
    }
    let Action::Send(outgoing) = app.key(Key::Enter) else {
        panic!("Enter should send");
    };
    assert_eq!(outgoing.to, KLODIK, "Klodik is an ordinary peer");
    let id = send(iva, &outgoing).unwrap();
    app.finish_send(Ok(id.clone()));
    app.begin_refresh();
    app.finish(Ok(load(iva).unwrap()));
    id
}

fn auto_sync(app: &mut App, iva: &Shell<&MemoryRelay>) {
    assert!(app.begin_auto_sync(Instant::now()), "no sync in flight");
    app.finish(Ok(load(iva).unwrap()));
}

#[test]
fn tui_message_to_klodik_comes_back_in_the_same_conversation() {
    let relay = MemoryRelay::default();
    let (iva, klodik, state) = setup("chat", &relay);
    let mut app = App::new();
    app.begin_refresh();
    app.finish(Ok(load(&iva).unwrap()));
    assert_eq!(app.selected_peer().unwrap().id, KLODIK);
    app.key(Key::Enter);

    let sent = type_and_send(&mut app, &iva, "klodik, jesi ziv?");
    assert!(app.composing(), "compose stays open for the next message");
    let rows = app.thread(KLODIK);
    let [Row::Sent(out)] = rows.as_slice() else {
        panic!("{rows:?}");
    };
    assert!(out.acked_by.is_empty(), "no ACK before Klodik has it");

    let model = Fake {
        calls: Cell::new(0),
        answer: Ok("živ sam!"),
    };
    let mut peer = Klodik {
        shell: klodik,
        model,
        state: State::load(&state).unwrap(),
        allowed: vec![NodeId::parse(IVA).unwrap()],
        rooms: Vec::new(),
        may_ask: Vec::new(),
    };
    assert_eq!(peer.step().unwrap().replied.len(), 1);
    assert_eq!(peer.model.calls.get(), 1);

    // The ordinary background sync is enough; no `r`, no special path.
    auto_sync(&mut app, &iva);
    let rows = app.thread(KLODIK);
    let [Row::Sent(out), Row::Received(reply)] = rows.as_slice() else {
        panic!("{rows:?}");
    };
    assert_eq!(out.id, sent);
    assert_eq!(out.acked_by, [KLODIK], "ACK: Klodik received it");
    assert_eq!(reply.body, "živ sam!");
    assert_eq!(reply.correlation.as_deref(), Some(sent.as_str()));
    assert_eq!(
        app.selected_message().unwrap().id(),
        reply.id,
        "follows the reply"
    );
    assert!(app.composing());

    // A Klodik restart answers nothing twice.
    peer.state = State::load(&state).unwrap();
    peer.step().unwrap();
    assert_eq!(peer.model.calls.get(), 1);
    auto_sync(&mut app, &iva);
    assert_eq!(app.thread(KLODIK).len(), 2);
}

#[test]
fn ack_without_a_reply_is_shown_as_ack_only() {
    let relay = MemoryRelay::default();
    let (iva, klodik, state) = setup("ack-only", &relay);
    let mut app = App::new();
    app.begin_refresh();
    app.finish(Ok(load(&iva).unwrap()));
    app.key(Key::Enter);
    type_and_send(&mut app, &iva, "hello?");

    let mut peer = Klodik {
        shell: klodik,
        model: Fake {
            calls: Cell::new(0),
            answer: Err(ModelError::Empty),
        },
        state: State::load(&state).unwrap(),
        allowed: vec![NodeId::parse(IVA).unwrap()],
        rooms: Vec::new(),
        may_ask: Vec::new(),
    };
    peer.step().unwrap();
    auto_sync(&mut app, &iva);
    let rows = app.thread(KLODIK);
    let [Row::Sent(out)] = rows.as_slice() else {
        panic!("only the ACKed message, no reply: {rows:?}");
    };
    assert_eq!(out.acked_by, [KLODIK]);
}
