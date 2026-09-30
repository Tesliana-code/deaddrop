use std::fmt::Debug;

use deaddrop_protocol::{EnvelopeV0, MessageId, MessageKind, NodeId};
use deaddrop_store::{InMemoryMessageStore, MessageStore, MessageStoreOutcome, SqliteMessageStore};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn message_id(value: &str) -> MessageId {
    MessageId::parse(value).unwrap()
}

fn envelope(id: &str, to: &str) -> EnvelopeV0 {
    EnvelopeV0::new(
        message_id(id),
        node("node-a"),
        node(to),
        MessageKind::Message,
        format!("body of {id}"),
    )
}

fn ids(values: &[&str]) -> Vec<MessageId> {
    values.iter().map(|value| message_id(value)).collect()
}

/// Stored in an order deliberately unrelated to the listing order.
fn corpus() -> Vec<EnvelopeV0> {
    vec![
        envelope("msg-3", "node-b"),
        envelope("msg-c", "node-c"),
        envelope("msg-1", "node-b"),
        envelope("Msg-2", "node-b"),
        envelope("msg-é", "node-b"),
        envelope("msg-prefix", "node-b-suffix"),
        envelope("msg-case", "Node-B"),
        envelope("msg-2", "node-b"),
    ]
}

/// Byte-wise UTF-8 order: uppercase before lowercase, non-ASCII last.
const NODE_B_IDS: &[&str] = &["Msg-2", "msg-1", "msg-2", "msg-3", "msg-é"];

fn fill<S: MessageStore>(store: &mut S)
where
    S::Error: Debug,
{
    for item in corpus() {
        assert_eq!(store.store(item).unwrap(), MessageStoreOutcome::Stored);
    }
}

fn assert_discovery_contract<S: MessageStore>(store: &mut S)
where
    S::Error: Debug,
{
    // Empty store: empty collection, not an error.
    assert!(store.ids_for_recipient(&node("node-b")).unwrap().is_empty());

    fill(store);

    // Exact recipient only: node-c, node-b-suffix and Node-B are excluded.
    let listed = store.ids_for_recipient(&node("node-b")).unwrap();
    assert_eq!(listed, ids(NODE_B_IDS));
    assert_eq!(
        store.ids_for_recipient(&node("node-c")).unwrap(),
        ids(&["msg-c"])
    );
    assert_eq!(
        store.ids_for_recipient(&node("Node-B")).unwrap(),
        ids(&["msg-case"])
    );
    assert!(store.ids_for_recipient(&node("node")).unwrap().is_empty());
    assert!(store.ids_for_recipient(&node("node-z")).unwrap().is_empty());

    // Deterministic across repeated reads.
    assert_eq!(store.ids_for_recipient(&node("node-b")).unwrap(), listed);

    // Exact replay neither duplicates nor reorders discovery results.
    for item in corpus() {
        assert_eq!(
            store.store(item).unwrap(),
            MessageStoreOutcome::AlreadyPresent
        );
    }
    assert_eq!(store.ids_for_recipient(&node("node-b")).unwrap(), listed);

    // A conflicting reuse of an id adds nothing either.
    assert!(store.store(envelope("msg-1", "node-z")).is_err());
    assert_eq!(store.ids_for_recipient(&node("node-b")).unwrap(), listed);
    assert!(store.ids_for_recipient(&node("node-z")).unwrap().is_empty());

    // Every discovered id loads the exact stored envelope, and discovery
    // changed nothing about it.
    let expected: Vec<EnvelopeV0> = corpus();
    for id in &listed {
        let loaded = store.load(id).unwrap().expect("discovered id loads");
        assert_eq!(loaded.to(), &node("node-b"));
        assert!(expected.contains(&loaded));
    }
}

#[test]
fn in_memory_store_satisfies_recipient_discovery_contract() {
    assert_discovery_contract(&mut InMemoryMessageStore::new());
}

#[test]
fn sqlite_store_satisfies_recipient_discovery_contract() {
    assert_discovery_contract(&mut SqliteMessageStore::open_in_memory().unwrap());
}

#[test]
fn sqlite_matches_in_memory_recipient_discovery() {
    let mut memory = InMemoryMessageStore::new();
    let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();

    fill(&mut memory);
    fill(&mut sqlite);

    for recipient in ["node-b", "node-c", "Node-B", "node-b-suffix", "node-z"] {
        assert_eq!(
            memory.ids_for_recipient(&node(recipient)).unwrap(),
            sqlite.ids_for_recipient(&node(recipient)).unwrap(),
            "recipient {recipient}"
        );
    }
}

#[test]
fn sqlite_discovery_survives_reopen() {
    let dir = std::env::temp_dir().join(format!(
        "deaddrop-recipient-discovery-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reopen.sqlite3");
    let _ = std::fs::remove_file(&path);

    {
        let mut store = SqliteMessageStore::open(&path).unwrap();
        fill(&mut store);
    }

    let store = SqliteMessageStore::open(&path).unwrap();
    assert_eq!(
        store.ids_for_recipient(&node("node-b")).unwrap(),
        ids(NODE_B_IDS)
    );

    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
