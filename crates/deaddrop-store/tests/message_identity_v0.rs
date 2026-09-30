//! Message Identity V0: the identity tuple is MessageId alone.

use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, decode_envelope_v0,
    encode_envelope_v0,
};
use deaddrop_store::{
    InMemoryMessageStore, InMemoryMessageStoreError, MessageStore, MessageStoreOutcome,
    SqliteMessageStore, SqliteMessageStoreError,
};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn artifact(hex: char) -> ArtifactRef {
    ArtifactRef::from_str(&format!("sha256:{}", hex.to_string().repeat(64))).unwrap()
}

fn base(id: &MessageId) -> EnvelopeV0 {
    EnvelopeV0::new(
        id.clone(),
        node("node-a:agent:deaddrop"),
        node("node-b:agent:deaddrop"),
        MessageKind::Handoff,
        "continue this task",
    )
    .with_correlation_id(CorrelationId::parse("corr-1").unwrap())
    .with_artifact_ref(artifact('a'))
}

/// Envelopes sharing `id` that each differ from `base` in exactly one field.
fn variants(id: &MessageId) -> Vec<(&'static str, EnvelopeV0)> {
    let with = |from: &str, to: &str, kind, body: &str| {
        EnvelopeV0::new(id.clone(), node(from), node(to), kind, body)
    };
    let a = "node-a:agent:deaddrop";
    let b = "node-b:agent:deaddrop";
    let corr = || CorrelationId::parse("corr-1").unwrap();

    vec![
        (
            "from",
            with(
                "node-c:agent:deaddrop",
                b,
                MessageKind::Handoff,
                "continue this task",
            )
            .with_correlation_id(corr())
            .with_artifact_ref(artifact('a')),
        ),
        (
            "to",
            with(
                a,
                "node-c:agent:deaddrop",
                MessageKind::Handoff,
                "continue this task",
            )
            .with_correlation_id(corr())
            .with_artifact_ref(artifact('a')),
        ),
        (
            "kind",
            with(a, b, MessageKind::Request, "continue this task")
                .with_correlation_id(corr())
                .with_artifact_ref(artifact('a')),
        ),
        (
            "correlation_id",
            with(a, b, MessageKind::Handoff, "continue this task")
                .with_correlation_id(CorrelationId::parse("corr-2").unwrap())
                .with_artifact_ref(artifact('a')),
        ),
        (
            "correlation_id absent",
            with(a, b, MessageKind::Handoff, "continue this task").with_artifact_ref(artifact('a')),
        ),
        (
            "body",
            with(a, b, MessageKind::Handoff, "a different body")
                .with_correlation_id(corr())
                .with_artifact_ref(artifact('a')),
        ),
        (
            "artifact_refs",
            with(a, b, MessageKind::Handoff, "continue this task")
                .with_correlation_id(corr())
                .with_artifact_ref(artifact('b')),
        ),
        (
            "artifact_refs duplicated",
            with(a, b, MessageKind::Handoff, "continue this task")
                .with_correlation_id(corr())
                .with_artifact_ref(artifact('a'))
                .with_artifact_ref(artifact('a')),
        ),
    ]
}

#[test]
fn e_exact_replay_keeps_id_and_is_already_present() {
    let id = MessageId::generate();
    let original = base(&id);
    let replay = decode_envelope_v0(&encode_envelope_v0(&original).unwrap()).unwrap();
    assert_eq!(replay.id(), &id);

    let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();
    let mut memory = InMemoryMessageStore::new();

    assert_eq!(
        sqlite.store(original.clone()).unwrap(),
        MessageStoreOutcome::Stored
    );
    assert_eq!(
        memory.store(original.clone()).unwrap(),
        MessageStoreOutcome::Stored
    );
    assert_eq!(
        sqlite.store(replay.clone()).unwrap(),
        MessageStoreOutcome::AlreadyPresent
    );
    assert_eq!(
        memory.store(replay).unwrap(),
        MessageStoreOutcome::AlreadyPresent
    );
    assert_eq!(sqlite.load(&id).unwrap(), Some(original));
}

#[test]
fn f_same_id_with_any_different_field_conflicts() {
    let id = MessageId::generate();

    for (field, variant) in variants(&id) {
        let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();
        let mut memory = InMemoryMessageStore::new();
        sqlite.store(base(&id)).unwrap();
        memory.store(base(&id)).unwrap();

        assert!(
            matches!(
                sqlite.store(variant.clone()),
                Err(SqliteMessageStoreError::MessageIdentityConflict { ref message_id })
                    if *message_id == id
            ),
            "sqlite: {field}"
        );
        assert!(
            matches!(
                memory.store(variant),
                Err(InMemoryMessageStoreError::MessageIdentityConflict { .. })
            ),
            "memory: {field}"
        );
        assert_eq!(sqlite.load(&id).unwrap(), Some(base(&id)), "{field}");
    }
}

#[test]
fn g_different_senders_with_same_id_conflict() {
    let id = MessageId::parse("shared-id").unwrap();
    let from_a = EnvelopeV0::new(
        id.clone(),
        node("node-a:agent:deaddrop"),
        node("node-z:agent:deaddrop"),
        MessageKind::Message,
        "same body",
    );
    let from_b = EnvelopeV0::new(
        id.clone(),
        node("node-b:agent:deaddrop"),
        node("node-z:agent:deaddrop"),
        MessageKind::Message,
        "same body",
    );

    let mut sqlite = SqliteMessageStore::open_in_memory().unwrap();
    let mut memory = InMemoryMessageStore::new();
    sqlite.store(from_a.clone()).unwrap();
    memory.store(from_a.clone()).unwrap();

    assert!(matches!(
        sqlite.store(from_b.clone()),
        Err(SqliteMessageStoreError::MessageIdentityConflict { .. })
    ));
    assert!(matches!(
        memory.store(from_b),
        Err(InMemoryMessageStoreError::MessageIdentityConflict { .. })
    ));
    assert_eq!(sqlite.load(&id).unwrap(), Some(from_a));
}
