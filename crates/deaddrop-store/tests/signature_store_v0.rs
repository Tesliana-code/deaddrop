//! Detached signature records in the SQLite message store.
//!
//! The store keeps evidence only: it never verifies, ranks, or chooses among
//! signature records. Several records may exist for one message id.

use std::path::PathBuf;

use deaddrop_protocol::{Ed25519PublicKey, MessageId, NodeId, SignatureV0};
use deaddrop_store::{MessageStore, MessageStoreOutcome, SqliteMessageStore};

fn record(message: &str, signer: &str, key: u8, sig: u8) -> SignatureV0 {
    SignatureV0::new(
        MessageId::parse(message).unwrap(),
        NodeId::parse(signer).unwrap(),
        Ed25519PublicKey::from_bytes([key; 32]),
        [sig; 64],
    )
}

fn mid(value: &str) -> MessageId {
    MessageId::parse(value).unwrap()
}

#[test]
fn exact_replay_is_already_present() {
    let mut store = SqliteMessageStore::open_in_memory().unwrap();
    let sig = record("msg-1", "node-a", 1, 2);
    assert_eq!(
        store.store_signature(&sig).unwrap(),
        MessageStoreOutcome::Stored
    );
    assert_eq!(
        store.store_signature(&sig).unwrap(),
        MessageStoreOutcome::AlreadyPresent
    );
    assert_eq!(store.signatures_for(&mid("msg-1")).unwrap(), vec![sig]);
}

#[test]
fn distinct_records_for_one_message_are_all_kept_in_deterministic_order() {
    let mut store = SqliteMessageStore::open_in_memory().unwrap();
    let a = record("msg-1", "node-b", 3, 4);
    let b = record("msg-1", "node-a", 1, 2);
    let c = record("msg-1", "node-a", 1, 9);
    for sig in [&a, &b, &c] {
        assert_eq!(
            store.store_signature(sig).unwrap(),
            MessageStoreOutcome::Stored
        );
    }
    assert_eq!(store.signatures_for(&mid("msg-1")).unwrap(), vec![b, c, a]);
}

#[test]
fn records_are_scoped_to_their_message_id_and_need_no_envelope() {
    let mut store = SqliteMessageStore::open_in_memory().unwrap();
    store
        .store_signature(&record("msg-1", "node-a", 1, 2))
        .unwrap();
    assert!(store.signatures_for(&mid("msg-2")).unwrap().is_empty());
    assert_eq!(store.load(&mid("msg-1")).unwrap(), None);
}

#[test]
fn records_survive_reopen() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("deaddrop-signature-store");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reopen.sqlite3");
    let _ = std::fs::remove_file(&path);
    let sig = record("msg-1", "node-a", 1, 2);
    SqliteMessageStore::open(&path)
        .unwrap()
        .store_signature(&sig)
        .unwrap();
    assert_eq!(
        SqliteMessageStore::open(&path)
            .unwrap()
            .signatures_for(&mid("msg-1"))
            .unwrap(),
        vec![sig]
    );
}
