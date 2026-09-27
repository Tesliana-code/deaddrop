use deaddrop_core::DeliveryProjection;
use deaddrop_protocol::{DeliveryEvent, DeliveryEventId, DeliveryEventKind, MessageId, NodeId};
use deaddrop_store::{AppendOutcome, DeliveryEventStore, InMemoryDeliveryEventStore};

fn event_id(value: &str) -> DeliveryEventId {
    DeliveryEventId::parse(value).expect("valid event id")
}

fn message_id(value: &str) -> MessageId {
    MessageId::parse(value).expect("valid message id")
}

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node id")
}

fn event(id: &str, message: &str, reporter: &str, kind: DeliveryEventKind) -> DeliveryEvent {
    DeliveryEvent::new(event_id(id), message_id(message), node(reporter), kind)
}

#[test]
fn persisted_evidence_rebuilds_delivery_projection_without_losing_history() {
    let mut store = InMemoryDeliveryEventStore::new();

    let failure = event("e1", "msg-1", "relay-a", DeliveryEventKind::DeliveryFailed);

    assert_eq!(store.append(failure.clone()), Ok(AppendOutcome::Appended));

    assert_eq!(store.append(failure), Ok(AppendOutcome::AlreadyPresent));

    assert_eq!(
        store.append(event(
            "e2",
            "msg-1",
            "relay-b",
            DeliveryEventKind::TransportAccepted,
        )),
        Ok(AppendOutcome::Appended)
    );

    assert_eq!(
        store.append(event(
            "e3",
            "msg-1",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        )),
        Ok(AppendOutcome::Appended)
    );

    assert_eq!(
        store.append(event(
            "e4",
            "msg-1",
            "node-b",
            DeliveryEventKind::RecipientVerified,
        )),
        Ok(AppendOutcome::Appended)
    );

    assert_eq!(
        store.append(event(
            "e5",
            "msg-1",
            "node-b",
            DeliveryEventKind::RecipientAcknowledged,
        )),
        Ok(AppendOutcome::Appended)
    );

    // Unrelated evidence must remain outside msg-1's reconstruction.
    store
        .append(event(
            "other-1",
            "msg-other",
            "relay-z",
            DeliveryEventKind::DeliveryFailed,
        ))
        .expect("foreign message append should succeed");

    let persisted = store
        .load_for_message(&message_id("msg-1"))
        .expect("message evidence should load");

    assert_eq!(persisted.len(), 5);

    let projection = DeliveryProjection::from_events(message_id("msg-1"), &persisted)
        .expect("persisted evidence should rebuild");

    assert_eq!(projection.failure_observations(), 1);
    assert_eq!(projection.transport_acceptances(), 1);
    assert!(projection.recipient_received());
    assert!(projection.recipient_verified());
    assert!(projection.recipient_acknowledged());
    assert_eq!(projection.expiry_observations(), 0);
}

#[test]
fn rebuilding_projection_twice_from_same_persisted_evidence_is_stable() {
    let mut store = InMemoryDeliveryEventStore::new();

    store
        .append(event(
            "e1",
            "msg-2",
            "relay-a",
            DeliveryEventKind::TransportAccepted,
        ))
        .expect("append should succeed");

    store
        .append(event(
            "e2",
            "msg-2",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        ))
        .expect("append should succeed");

    let persisted = store
        .load_for_message(&message_id("msg-2"))
        .expect("message evidence should load");

    let first = DeliveryProjection::from_events(message_id("msg-2"), &persisted)
        .expect("first rebuild should succeed");

    let second = DeliveryProjection::from_events(message_id("msg-2"), &persisted)
        .expect("second rebuild should succeed");

    assert_eq!(first, second);
}
