//! Stores refuse EnvelopeV0 values that exceed the V0 protocol limits, however
//! they were constructed, and fail closed on persisted rows that exceed them.

use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, EnvelopeLimitError, EnvelopeV0, MAX_ARTIFACT_REFS, MAX_CANONICAL_ENVELOPE_BYTES,
    MessageId, MessageKind, NodeId, encode_envelope_v0,
};
use deaddrop_store::{
    InMemoryMessageStore, InMemoryMessageStoreError, MessageStore, MessageStoreOutcome,
    SqliteMessageStore, SqliteMessageStoreError,
};

fn envelope(id: &str, body: &str) -> EnvelopeV0 {
    EnvelopeV0::new(
        MessageId::parse(id).unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse("node-b").unwrap(),
        MessageKind::Message,
        body,
    )
}

fn with_refs(mut envelope: EnvelopeV0, count: usize) -> EnvelopeV0 {
    for n in 0..count {
        envelope =
            envelope.with_artifact_ref(ArtifactRef::from_str(&format!("sha256:{n:064x}")).unwrap());
    }
    envelope
}

/// Envelope whose canonical encoding is exactly `len` bytes.
fn envelope_of_len(id: &str, len: usize) -> EnvelopeV0 {
    let framing = encode_envelope_v0(&envelope(id, "")).unwrap().len();
    envelope(id, &"x".repeat(len - framing))
}

fn violations() -> [(EnvelopeV0, EnvelopeLimitError); 2] {
    [
        (
            envelope_of_len("msg-too-large", MAX_CANONICAL_ENVELOPE_BYTES + 1),
            EnvelopeLimitError::EnvelopeTooLarge {
                len: MAX_CANONICAL_ENVELOPE_BYTES + 1,
            },
        ),
        (
            with_refs(envelope("msg-too-many-refs", ""), MAX_ARTIFACT_REFS + 1),
            EnvelopeLimitError::TooManyArtifactRefs {
                count: MAX_ARTIFACT_REFS + 1,
            },
        ),
    ]
}

fn at_limits() -> [EnvelopeV0; 2] {
    [
        envelope_of_len("msg-exact-size", MAX_CANONICAL_ENVELOPE_BYTES),
        with_refs(envelope("msg-exact-refs", ""), MAX_ARTIFACT_REFS),
    ]
}

// J
#[test]
fn j_in_memory_store_rejects_violating_envelope() {
    let mut store = InMemoryMessageStore::new();

    for (violating, expected) in violations() {
        let id = violating.id().clone();
        assert_eq!(
            store.store(violating),
            Err(InMemoryMessageStoreError::EnvelopeLimit(expected))
        );
        assert_eq!(store.load(&id).unwrap(), None);
    }

    for exact in at_limits() {
        assert_eq!(
            store.store(exact.clone()).unwrap(),
            MessageStoreOutcome::Stored
        );
        assert_eq!(store.load(exact.id()).unwrap(), Some(exact));
    }
}

// K
#[test]
fn k_sqlite_store_rejects_violating_envelope() {
    let mut store = SqliteMessageStore::open_in_memory().unwrap();

    for (violating, expected) in violations() {
        let id = violating.id().clone();
        match store.store(violating) {
            Err(SqliteMessageStoreError::EnvelopeLimit(actual)) => assert_eq!(actual, expected),
            other => panic!("expected {expected:?}, got {other:?}"),
        }
        assert!(store.load(&id).unwrap().is_none());
    }

    for exact in at_limits() {
        assert_eq!(
            store.store(exact.clone()).unwrap(),
            MessageStoreOutcome::Stored
        );
        assert_eq!(store.load(exact.id()).unwrap(), Some(exact));
    }
}

// K: rows written around the store (legacy or tampered) fail closed on load.
#[test]
fn k_sqlite_load_fails_closed_on_persisted_violation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.sqlite3");

    let mut store = SqliteMessageStore::open(&path).unwrap();
    store
        .store(with_refs(envelope("msg-refs", ""), MAX_ARTIFACT_REFS))
        .unwrap();
    store.store(envelope("msg-body", "")).unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO message_artifact_refs (message_id, ordinal, artifact_ref) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            "msg-refs",
            MAX_ARTIFACT_REFS as i64,
            format!("sha256:{:064x}", 0)
        ],
    )
    .unwrap();
    raw.execute(
        "UPDATE messages SET body = ?1 WHERE message_id = 'msg-body'",
        rusqlite::params!["x".repeat(MAX_CANONICAL_ENVELOPE_BYTES)],
    )
    .unwrap();

    match store.load(&MessageId::parse("msg-refs").unwrap()) {
        Err(SqliteMessageStoreError::EnvelopeLimit(EnvelopeLimitError::TooManyArtifactRefs {
            count,
        })) => assert_eq!(count, MAX_ARTIFACT_REFS + 1),
        other => panic!("expected ref-count violation, got {other:?}"),
    }

    match store.load(&MessageId::parse("msg-body").unwrap()) {
        Err(SqliteMessageStoreError::EnvelopeLimit(EnvelopeLimitError::EnvelopeTooLarge {
            len,
        })) => assert!(len > MAX_CANONICAL_ENVELOPE_BYTES),
        other => panic!("expected size violation, got {other:?}"),
    }
}

#[test]
fn persisted_oversized_identifier_fails_closed_without_echoing_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.sqlite3");

    let mut store = SqliteMessageStore::open(&path).unwrap();
    store.store(envelope("msg-sender", "")).unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute(
        "UPDATE messages SET sender = ?1 WHERE message_id = 'msg-sender'",
        rusqlite::params!["s".repeat(10_000)],
    )
    .unwrap();

    let error = store
        .load(&MessageId::parse("msg-sender").unwrap())
        .unwrap_err();
    assert!(matches!(
        error,
        SqliteMessageStoreError::InvalidPersistedField {
            field: "sender",
            ..
        }
    ));
    assert!(error.to_string().len() < 200, "{error}");
}
