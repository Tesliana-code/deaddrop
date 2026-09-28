use std::fs;
use std::str::FromStr;

use deaddrop_protocol::{ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId};
use deaddrop_store::{InMemoryMessageStore, MessageStore, MessageStoreOutcome, SqliteMessageStore};

fn envelope(id: &str, body: &str) -> EnvelopeV0 {
    EnvelopeV0::new(
        MessageId::parse(id).unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse("node-b").unwrap(),
        MessageKind::Handoff,
        body,
    )
    .with_correlation_id(CorrelationId::parse("corr-1").unwrap())
    .with_artifact_ref(
        ArtifactRef::from_str(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap(),
    )
    .with_artifact_ref(
        ArtifactRef::from_str(
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        )
        .unwrap(),
    )
}

#[test]
fn sqlite_matches_in_memory_message_semantics() {
    let mut memory = InMemoryMessageStore::new();
    let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();

    let message = envelope("msg-parity", "exact envelope");

    assert_eq!(
        memory.store(message.clone()).unwrap(),
        MessageStoreOutcome::Stored
    );

    assert_eq!(
        sqlite.store(message.clone()).unwrap(),
        MessageStoreOutcome::Stored
    );

    assert_eq!(
        memory.store(message.clone()).unwrap(),
        MessageStoreOutcome::AlreadyPresent
    );

    assert_eq!(
        sqlite.store(message.clone()).unwrap(),
        MessageStoreOutcome::AlreadyPresent
    );

    let message_id = MessageId::parse("msg-parity").unwrap();

    let memory_loaded = memory.load(&message_id).unwrap();
    let sqlite_loaded = sqlite.load(&message_id).unwrap();

    assert_eq!(memory_loaded, Some(message.clone()));
    assert_eq!(sqlite_loaded, Some(message));
    assert_eq!(memory_loaded, sqlite_loaded);
}

#[test]
fn sqlite_detects_same_id_with_different_envelope() {
    let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();

    sqlite.store(envelope("collision", "first body")).unwrap();

    let result = sqlite.store(envelope("collision", "different body"));

    assert!(matches!(
        result,
        Err(deaddrop_store::SqliteMessageStoreError::MessageIdentityConflict { .. })
    ));
}

#[test]
fn sqlite_message_survives_close_and_reopen() {
    let path = std::env::temp_dir().join(format!(
        "deaddrop-message-store-reopen-{}.db",
        std::process::id()
    ));

    let _ = fs::remove_file(&path);

    let message = envelope("msg-reopen", "persistent semantic envelope");

    {
        let mut store = SqliteMessageStore::open(&path).unwrap();

        assert_eq!(
            store.store(message.clone()).unwrap(),
            MessageStoreOutcome::Stored
        );
    }

    {
        let store = SqliteMessageStore::open(&path).unwrap();

        let loaded = store
            .load(&MessageId::parse("msg-reopen").unwrap())
            .unwrap();

        assert_eq!(loaded, Some(message));
    }

    fs::remove_file(&path).unwrap();
}
