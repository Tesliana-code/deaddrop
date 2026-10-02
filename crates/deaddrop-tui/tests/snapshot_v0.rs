//! `snapshot::load` reads node, peers, inbox, and ACKs through the real
//! `Shell` over the in-process `MemoryRelay`.

use std::path::{Path, PathBuf};

use deaddrop_protocol::{ArtifactRef, EnvelopeV0, MessageId, NodeId, SignatureV0};
use deaddrop_shell::{MemoryRelay, Relay, RelayError, Shell, init};
use deaddrop_tui::app::{App, Row};
use deaddrop_tui::snapshot::load;

const A: &str = "node-a:tui:deaddrop";
const B: &str = "node-b:tui:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn home(test: &str, who: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-tui-tests")
        .join(test)
        .join(who);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    dir
}

/// Two nodes on one relay that trust each other.
fn pair<'r>(
    test: &str,
    relay: &'r MemoryRelay,
) -> (Shell<&'r MemoryRelay>, Shell<&'r MemoryRelay>) {
    let open = |who: &str| {
        let dir = home(test, who);
        init(&dir, node(who), RELAY).unwrap();
        Shell::open_with(&dir, relay).unwrap()
    };
    let (a, b) = (open(A), open(B));
    let (ca, cb) = (a.identity(), b.identity());
    a.add_peer(&cb.node, &cb.key).unwrap();
    b.add_peer(&ca.node, &ca.key).unwrap();
    (a, b)
}

#[test]
fn fresh_node_loads_empty() {
    let relay = MemoryRelay::default();
    let dir = home("fresh", A);
    init(&dir, node(A), RELAY).unwrap();
    let snapshot = load(&Shell::open_with(&dir, &relay).unwrap()).unwrap();
    assert_eq!(snapshot.node.id, A);
    assert_eq!(snapshot.node.relay, RELAY);
    assert!(snapshot.peers.is_empty() && snapshot.inbox.is_empty());
    assert_eq!(snapshot.sync.unwrap().received, 0);
}

#[test]
fn load_syncs_inbox_and_acks() {
    let relay = MemoryRelay::default();
    let (a, b) = pair("roundtrip", &relay);
    let sent = a.send(&node(B), "hello\nsecond line", None, &[]).unwrap();

    let at_b = load(&b).unwrap();
    assert_eq!(at_b.peers.len(), 1);
    assert_eq!(at_b.peers[0].id, A);
    let [message] = at_b.inbox.as_slice() else {
        panic!("one message expected");
    };
    assert_eq!(message.id, sent.as_str());
    assert_eq!(message.from, A);
    assert_eq!(message.body, "hello\nsecond line");
    assert_eq!(
        message.delivery,
        ["recipient_received", "recipient_verified"]
    );
    assert_eq!(at_b.sync.as_ref().unwrap().received, 1);

    assert!(at_b.sent.is_empty(), "received is not sent");

    let at_a = load(&a).unwrap();
    assert!(at_a.inbox.is_empty(), "sent is not received");
    let [out] = at_a.sent.as_slice() else {
        panic!("one sent message expected");
    };
    assert_eq!(out.id, sent.as_str());
    assert_eq!(out.to, B);
    assert!(out.acked_by.is_empty());
}

#[test]
fn ack_status_survives_restart_without_duplicate_rows() {
    let relay = MemoryRelay::default();
    let (a, b) = pair("ack-restart", &relay);
    let sent = a.send(&node(B), "hi", None, &[]).unwrap();
    b.sync().unwrap();
    b.ack(&sent).unwrap();
    b.ack(&sent).unwrap();
    a.sync().unwrap();
    a.sync().unwrap();

    // Fresh Shell and fresh App over the same home, as after a restart.
    let a = Shell::open_with(&home_of(A, "ack-restart"), &relay).unwrap();
    let mut app = App::new();
    app.begin_refresh();
    app.finish(Ok(load(&a).unwrap()));
    let rows = app.thread(B);
    let [Row::Sent(row)] = rows.as_slice() else {
        panic!("one sent row expected, got {rows:?}");
    };
    assert_eq!(row.id, sent.as_str());
    assert_eq!(row.acked_by, [B]);
}

/// A relay that is down.
struct Down;

impl Relay for Down {
    fn post_message(&self, _: &EnvelopeV0) -> Result<(), RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn post_signature(&self, _: &SignatureV0) -> Result<(), RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn list_for(&self, _: &NodeId) -> Result<Vec<MessageId>, RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn get_message(&self, _: &MessageId) -> Result<Option<EnvelopeV0>, RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn signatures(&self, _: &MessageId) -> Result<Vec<SignatureV0>, RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn put_artifact(&self, _: &[u8]) -> Result<ArtifactRef, RelayError> {
        Err(RelayError::Failed("down".into()))
    }
    fn get_artifact(&self, _: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError> {
        Err(RelayError::Failed("down".into()))
    }
}

#[test]
fn relay_down_still_shows_local_state() {
    let relay = MemoryRelay::default();
    let (a, b) = pair("offline", &relay);
    a.send(&node(B), "kept locally", None, &[]).unwrap();
    load(&b).unwrap();

    let dir: &Path = &home_of(B, "offline");
    let snapshot = load(&Shell::open_with(dir, Down).unwrap()).unwrap();
    assert!(snapshot.sync.unwrap_err().contains("down"));
    assert_eq!(snapshot.peers.len(), 1);
    assert_eq!(snapshot.inbox.len(), 1);
}

#[test]
fn relay_down_still_shows_sent_and_ack_status() {
    let relay = MemoryRelay::default();
    let (a, b) = pair("offline-sent", &relay);
    let sent = a.send(&node(B), "hi", None, &[]).unwrap();
    b.sync().unwrap();
    b.ack(&sent).unwrap();
    a.sync().unwrap();

    let snapshot = load(&Shell::open_with(&home_of(A, "offline-sent"), Down).unwrap()).unwrap();
    assert!(snapshot.sync.is_err());
    assert_eq!(snapshot.sent.len(), 1);
    assert_eq!(snapshot.sent[0].acked_by, [B]);
}

fn home_of(who: &str, test: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-tui-tests")
        .join(test)
        .join(who)
}
