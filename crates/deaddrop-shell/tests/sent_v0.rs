//! `Shell::sent` lists what this node sent, and ACK status is read back from
//! durable delivery evidence, so both survive reopening the node.

use std::path::{Path, PathBuf};

use deaddrop_protocol::{ArtifactRef, CorrelationId, DeliveryEventKind, MessageKind, NodeId};
use deaddrop_shell::{MemoryRelay, Shell, init};

const A: &str = "node-a:sent:deaddrop";
const B: &str = "node-b:sent:deaddrop";
const C: &str = "node-c:sent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn home(test: &str, who: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-shell-sent-tests")
        .join(test)
        .join(who);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    dir
}

fn open<'r>(dir: &Path, relay: &'r MemoryRelay) -> Shell<&'r MemoryRelay> {
    Shell::open_with(dir, relay).unwrap()
}

/// Nodes on one relay that all trust each other. Returns their homes.
fn mesh(test: &str, relay: &MemoryRelay, nodes: &[&str]) -> Vec<PathBuf> {
    let dirs: Vec<_> = nodes
        .iter()
        .map(|name| {
            let dir = home(test, name);
            init(&dir, node(name), RELAY).unwrap();
            dir
        })
        .collect();
    for truster in &dirs {
        for trusted in &dirs {
            if truster != trusted {
                let card = open(trusted, relay).identity();
                open(truster, relay)
                    .add_peer(&card.node, &card.key)
                    .unwrap();
            }
        }
    }
    dirs
}

fn acked(shell: &Shell<&MemoryRelay>, id: &deaddrop_protocol::MessageId) -> Vec<NodeId> {
    shell
        .delivery(id)
        .unwrap()
        .into_iter()
        .filter(|e| e.kind() == DeliveryEventKind::RecipientAcknowledged)
        .map(|e| e.reported_by().clone())
        .collect()
}

#[test]
fn fresh_node_has_sent_nothing() {
    let relay = MemoryRelay::default();
    let dirs = mesh("fresh", &relay, &[A, B]);
    assert!(open(&dirs[0], &relay).sent().unwrap().is_empty());
}

#[test]
fn sent_lists_outgoing_messages_with_their_fields() {
    let relay = MemoryRelay::default();
    let dirs = mesh("fields", &relay, &[A, B, C]);
    let a = open(&dirs[0], &relay);
    let artifact = a.put_artifact(b"bytes").unwrap();
    let to_b = a
        .send(
            &node(B),
            "for b",
            Some(CorrelationId::parse("thread-1").unwrap()),
            std::slice::from_ref(&artifact),
        )
        .unwrap();
    let to_c = a.send(&node(C), "for c", None, &[]).unwrap();

    let sent = a.sent().unwrap();
    let ids: Vec<_> = sent.iter().map(|m| m.id().clone()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&to_b) && ids.contains(&to_c));
    let b = sent.iter().find(|m| m.id() == &to_b).unwrap();
    assert_eq!(b.from(), &node(A));
    assert_eq!(b.to(), &node(B));
    assert_eq!(b.body(), "for b");
    assert_eq!(b.correlation_id().unwrap().as_str(), "thread-1");
    assert_eq!(b.artifact_refs(), &[artifact] as &[ArtifactRef]);
}

#[test]
fn incoming_and_outgoing_stay_distinct() {
    let relay = MemoryRelay::default();
    let dirs = mesh("directions", &relay, &[A, B]);
    let (a, b) = (open(&dirs[0], &relay), open(&dirs[1], &relay));
    let out = a.send(&node(B), "a to b", None, &[]).unwrap();
    let back = b.send(&node(A), "b to a", None, &[]).unwrap();
    a.sync().unwrap();
    b.sync().unwrap();

    let ids = |v: Vec<deaddrop_protocol::EnvelopeV0>| -> Vec<_> {
        v.into_iter().map(|m| m.id().clone()).collect()
    };
    assert_eq!(ids(a.sent().unwrap()), std::slice::from_ref(&out));
    assert_eq!(ids(a.inbox().unwrap()), std::slice::from_ref(&back));
    assert_eq!(ids(b.sent().unwrap()), [back]);
    assert_eq!(ids(b.inbox().unwrap()), [out]);
}

#[test]
fn acks_this_node_sends_are_not_listed_as_sent() {
    let relay = MemoryRelay::default();
    let dirs = mesh("own-acks", &relay, &[A, B]);
    let (a, b) = (open(&dirs[0], &relay), open(&dirs[1], &relay));
    let id = a.send(&node(B), "hi", None, &[]).unwrap();
    b.sync().unwrap();
    b.ack(&id).unwrap();

    assert!(b.sent().unwrap().is_empty());
    assert!(
        b.sent()
            .unwrap()
            .iter()
            .all(|m| m.kind() != MessageKind::Acknowledgment)
    );
}

#[test]
fn ack_status_survives_a_fresh_shell() {
    let relay = MemoryRelay::default();
    let dirs = mesh("ack-restart", &relay, &[A, B]);
    let id = open(&dirs[0], &relay)
        .send(&node(B), "hi", None, &[])
        .unwrap();
    let b = open(&dirs[1], &relay);
    b.sync().unwrap();
    b.ack(&id).unwrap();
    open(&dirs[0], &relay).sync().unwrap();

    // A brand-new Shell over the same home, no sync: status is durable.
    let a = open(&dirs[0], &relay);
    assert_eq!(a.sent().unwrap().len(), 1);
    assert_eq!(acked(&a, &id), [node(B)]);
}

#[test]
fn replayed_ack_is_recorded_once() {
    let relay = MemoryRelay::default();
    let dirs = mesh("ack-replay", &relay, &[A, B]);
    let a = open(&dirs[0], &relay);
    let id = a.send(&node(B), "hi", None, &[]).unwrap();
    let b = open(&dirs[1], &relay);
    b.sync().unwrap();
    b.ack(&id).unwrap();
    b.ack(&id).unwrap();
    a.sync().unwrap();
    a.sync().unwrap();

    let a = open(&dirs[0], &relay);
    assert_eq!(a.sent().unwrap().len(), 1);
    assert_eq!(acked(&a, &id), [node(B)]);
}
