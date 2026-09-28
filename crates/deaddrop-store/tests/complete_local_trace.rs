use std::fs;
use std::str::FromStr;

use deaddrop_core::DeliveryProjection;
use deaddrop_protocol::{
    ArtifactRef, CorrelationId, DeliveryEvent, DeliveryEventId, DeliveryEventKind, EnvelopeV0,
    MessageId, MessageKind, NodeId,
};
use deaddrop_store::{
    AppendOutcome, DeliveryEventStore, MessageStore, MessageStoreOutcome, SqliteDeliveryEventStore,
    SqliteMessageStore,
};

fn message_id(value: &str) -> MessageId {
    MessageId::parse(value).expect("valid message id")
}

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node id")
}

fn event(
    id: &str,
    message_id: &MessageId,
    reporter: &str,
    kind: DeliveryEventKind,
) -> DeliveryEvent {
    DeliveryEvent::new(
        DeliveryEventId::parse(id).expect("valid event id"),
        message_id.clone(),
        node(reporter),
        kind,
    )
}

#[test]
fn durable_message_and_delivery_evidence_rebuild_complete_local_trace() {
    let path = std::env::temp_dir().join(format!(
        "deaddrop-complete-local-trace-{}.db",
        std::process::id()
    ));

    let _ = fs::remove_file(&path);

    let id = message_id("msg-local-trace");

    let first_artifact = ArtifactRef::from_str(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .expect("valid artifact ref");

    let second_artifact = ArtifactRef::from_str(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("valid artifact ref");

    let envelope = EnvelopeV0::new(
        id.clone(),
        node("workstation-a:agent:deaddrop"),
        node("workstation-b:agent:deaddrop"),
        MessageKind::Handoff,
        "inspect the referenced artifacts and continue the task",
    )
    .with_correlation_id(CorrelationId::parse("corr-local-trace").expect("valid correlation id"))
    .with_artifact_ref(first_artifact.clone())
    .with_artifact_ref(second_artifact.clone());

    let events = vec![
        event(
            "evt-1",
            &id,
            "relay-a",
            DeliveryEventKind::TransportAccepted,
        ),
        event("evt-2", &id, "relay-a", DeliveryEventKind::DeliveryFailed),
        event(
            "evt-3",
            &id,
            "workstation-b:agent:deaddrop",
            DeliveryEventKind::RecipientReceived,
        ),
        event(
            "evt-4",
            &id,
            "workstation-b:agent:deaddrop",
            DeliveryEventKind::RecipientVerified,
        ),
        event(
            "evt-5",
            &id,
            "workstation-b:agent:deaddrop",
            DeliveryEventKind::RecipientAcknowledged,
        ),
    ];

    {
        let mut messages = SqliteMessageStore::open(&path).expect("message store should open");

        assert_eq!(
            messages
                .store(envelope.clone())
                .expect("message should persist"),
            MessageStoreOutcome::Stored
        );
    }

    {
        let mut delivery =
            SqliteDeliveryEventStore::open(&path).expect("delivery store should open");

        for event in &events {
            assert_eq!(
                delivery
                    .append(event.clone())
                    .expect("delivery evidence should persist"),
                AppendOutcome::Appended
            );
        }
    }

    // Both stores are closed here. The only surviving authority for this
    // local reconstruction is the durable SQLite file.
    {
        let messages = SqliteMessageStore::open(&path).expect("message store should reopen");

        let delivery = SqliteDeliveryEventStore::open(&path).expect("delivery store should reopen");

        let loaded_envelope = messages
            .load(&id)
            .expect("message load should succeed")
            .expect("message should exist");

        let loaded_events = delivery
            .load_for_message(&id)
            .expect("delivery evidence load should succeed");

        assert_eq!(loaded_envelope, envelope);
        assert_eq!(loaded_events, events);

        assert_eq!(
            loaded_envelope.from().as_str(),
            "workstation-a:agent:deaddrop"
        );
        assert_eq!(
            loaded_envelope.to().as_str(),
            "workstation-b:agent:deaddrop"
        );
        assert_eq!(
            loaded_envelope
                .correlation_id()
                .expect("correlation should survive")
                .as_str(),
            "corr-local-trace"
        );
        assert_eq!(
            loaded_envelope.artifact_refs(),
            &[first_artifact, second_artifact]
        );

        let projection = DeliveryProjection::from_events(id.clone(), &loaded_events)
            .expect("projection should rebuild");

        assert_eq!(projection.message_id(), &id);
        assert_eq!(projection.transport_acceptances(), 1);
        assert_eq!(projection.failure_observations(), 1);
        assert!(projection.recipient_received());
        assert!(projection.recipient_verified());
        assert!(projection.recipient_acknowledged());
    }

    fs::remove_file(&path).expect("temporary database should be removable");
}
