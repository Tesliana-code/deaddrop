use std::fs;

use deaddrop_core::DeliveryProjection;
use deaddrop_protocol::{DeliveryEvent, DeliveryEventId, DeliveryEventKind, MessageId, NodeId};
use deaddrop_store::{
    AppendOutcome, DeliveryEventStore, InMemoryDeliveryEventStore, SqliteDeliveryEventStore,
};

fn event(id: &str, message: &str, reporter: &str, kind: DeliveryEventKind) -> DeliveryEvent {
    DeliveryEvent::new(
        DeliveryEventId::parse(id).unwrap(),
        MessageId::parse(message).unwrap(),
        NodeId::parse(reporter).unwrap(),
        kind,
    )
}

#[test]
fn sqlite_matches_in_memory_replay_and_projection_semantics() {
    let mut memory = InMemoryDeliveryEventStore::new();
    let mut sqlite = SqliteDeliveryEventStore::open_in_memory().unwrap();

    let evidence = vec![
        event(
            "e1",
            "msg-parity",
            "relay-a",
            DeliveryEventKind::DeliveryFailed,
        ),
        event(
            "e2",
            "msg-parity",
            "relay-b",
            DeliveryEventKind::TransportAccepted,
        ),
        event(
            "e3",
            "msg-parity",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        ),
        event(
            "e4",
            "msg-parity",
            "node-b",
            DeliveryEventKind::RecipientVerified,
        ),
        event(
            "e5",
            "msg-parity",
            "node-b",
            DeliveryEventKind::RecipientAcknowledged,
        ),
    ];

    for item in &evidence {
        assert_eq!(
            memory.append(item.clone()).unwrap(),
            AppendOutcome::Appended
        );
        assert_eq!(
            sqlite.append(item.clone()).unwrap(),
            AppendOutcome::Appended
        );
    }

    assert_eq!(
        memory.append(evidence[0].clone()).unwrap(),
        AppendOutcome::AlreadyPresent
    );
    assert_eq!(
        sqlite.append(evidence[0].clone()).unwrap(),
        AppendOutcome::AlreadyPresent
    );

    let message_id = MessageId::parse("msg-parity").unwrap();

    let memory_events = memory.load_for_message(&message_id).unwrap();
    let sqlite_events = sqlite.load_for_message(&message_id).unwrap();

    assert_eq!(memory_events, sqlite_events);

    let memory_projection =
        DeliveryProjection::from_events(message_id.clone(), &memory_events).unwrap();

    let sqlite_projection = DeliveryProjection::from_events(message_id, &sqlite_events).unwrap();

    assert_eq!(memory_projection, sqlite_projection);

    assert_eq!(sqlite_projection.failure_observations(), 1);
    assert_eq!(sqlite_projection.transport_acceptances(), 1);
    assert!(sqlite_projection.recipient_received());
    assert!(sqlite_projection.recipient_verified());
    assert!(sqlite_projection.recipient_acknowledged());
}

#[test]
fn sqlite_evidence_survives_close_and_reopen() {
    let path =
        std::env::temp_dir().join(format!("deaddrop-sqlite-reopen-{}.db", std::process::id()));

    let _ = fs::remove_file(&path);

    {
        let mut store = SqliteDeliveryEventStore::open(&path).unwrap();

        store
            .append(event(
                "e1",
                "msg-reopen",
                "relay-a",
                DeliveryEventKind::TransportAccepted,
            ))
            .unwrap();

        store
            .append(event(
                "e2",
                "msg-reopen",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ))
            .unwrap();
    }

    {
        let store = SqliteDeliveryEventStore::open(&path).unwrap();

        let message_id = MessageId::parse("msg-reopen").unwrap();

        let persisted = store.load_for_message(&message_id).unwrap();

        assert_eq!(persisted.len(), 2);
        assert_eq!(persisted[0].id().as_str(), "e1");
        assert_eq!(persisted[1].id().as_str(), "e2");

        let projection = DeliveryProjection::from_events(message_id, &persisted).unwrap();

        assert_eq!(projection.transport_acceptances(), 1);
        assert!(projection.recipient_received());
    }

    fs::remove_file(&path).unwrap();
}
