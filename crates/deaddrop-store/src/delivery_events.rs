use std::collections::HashMap;
use std::fmt;

use deaddrop_protocol::{DeliveryEvent, DeliveryEventId, MessageId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InMemoryDeliveryEventStoreError {
    EventIdentityConflict { event_id: DeliveryEventId },
}

impl fmt::Display for InMemoryDeliveryEventStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EventIdentityConflict { event_id } => write!(
                f,
                "delivery event id {event_id} already identifies different evidence"
            ),
        }
    }
}

impl std::error::Error for InMemoryDeliveryEventStoreError {}

/// Persistence boundary for append-only delivery evidence.
///
/// A store persists protocol evidence. It does not derive delivery status,
/// task success, source authority, or other domain meaning.
pub trait DeliveryEventStore {
    type Error;

    fn append(&mut self, event: DeliveryEvent) -> Result<AppendOutcome, Self::Error>;

    fn load_for_message(&self, message_id: &MessageId) -> Result<Vec<DeliveryEvent>, Self::Error>;
}

/// Minimal reference implementation used to freeze store semantics before
/// selecting a durable database implementation.
#[derive(Debug, Default)]
pub struct InMemoryDeliveryEventStore {
    events_by_id: HashMap<DeliveryEventId, DeliveryEvent>,
    append_order: Vec<DeliveryEventId>,
}

impl InMemoryDeliveryEventStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeliveryEventStore for InMemoryDeliveryEventStore {
    type Error = InMemoryDeliveryEventStoreError;

    fn append(&mut self, event: DeliveryEvent) -> Result<AppendOutcome, Self::Error> {
        if let Some(existing) = self.events_by_id.get(event.id()) {
            if existing == &event {
                return Ok(AppendOutcome::AlreadyPresent);
            }

            return Err(InMemoryDeliveryEventStoreError::EventIdentityConflict {
                event_id: event.id().clone(),
            });
        }

        let event_id = event.id().clone();

        self.events_by_id.insert(event_id.clone(), event);
        self.append_order.push(event_id);

        Ok(AppendOutcome::Appended)
    }

    fn load_for_message(&self, message_id: &MessageId) -> Result<Vec<DeliveryEvent>, Self::Error> {
        let events = self
            .append_order
            .iter()
            .filter_map(|event_id| self.events_by_id.get(event_id))
            .filter(|event| event.message_id() == message_id)
            .cloned()
            .collect();

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deaddrop_protocol::{DeliveryEventKind, NodeId};

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
    fn appends_and_reads_message_scoped_evidence() {
        let mut store = InMemoryDeliveryEventStore::new();

        let first = event(
            "e1",
            "msg-1",
            "relay-a",
            DeliveryEventKind::TransportAccepted,
        );

        let second = event(
            "e2",
            "msg-1",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        );

        assert_eq!(store.append(first.clone()), Ok(AppendOutcome::Appended));

        assert_eq!(store.append(second.clone()), Ok(AppendOutcome::Appended));

        let loaded = store
            .load_for_message(&message_id("msg-1"))
            .expect("load should succeed");

        assert_eq!(loaded, vec![first, second]);
    }

    #[test]
    fn exact_replay_is_idempotent_at_persistence_boundary() {
        let mut store = InMemoryDeliveryEventStore::new();

        let evidence = event("e1", "msg-2", "relay-a", DeliveryEventKind::DeliveryFailed);

        assert_eq!(store.append(evidence.clone()), Ok(AppendOutcome::Appended));

        assert_eq!(
            store.append(evidence.clone()),
            Ok(AppendOutcome::AlreadyPresent)
        );

        let loaded = store
            .load_for_message(&message_id("msg-2"))
            .expect("load should succeed");

        assert_eq!(loaded, vec![evidence]);
    }

    #[test]
    fn conflicting_event_identity_fails_closed() {
        let mut store = InMemoryDeliveryEventStore::new();

        let original = event(
            "collision-1",
            "msg-3",
            "relay-a",
            DeliveryEventKind::DeliveryFailed,
        );

        let conflicting = event(
            "collision-1",
            "msg-3",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        );

        assert_eq!(store.append(original.clone()), Ok(AppendOutcome::Appended));

        assert_eq!(
            store.append(conflicting),
            Err(InMemoryDeliveryEventStoreError::EventIdentityConflict {
                event_id: event_id("collision-1"),
            })
        );

        let loaded = store
            .load_for_message(&message_id("msg-3"))
            .expect("load should succeed");

        assert_eq!(loaded, vec![original]);
    }

    #[test]
    fn reads_do_not_cross_message_boundaries() {
        let mut store = InMemoryDeliveryEventStore::new();

        store
            .append(event(
                "e1",
                "msg-a",
                "relay-a",
                DeliveryEventKind::TransportAccepted,
            ))
            .expect("append should succeed");

        store
            .append(event(
                "e2",
                "msg-b",
                "relay-a",
                DeliveryEventKind::TransportAccepted,
            ))
            .expect("append should succeed");

        let a = store
            .load_for_message(&message_id("msg-a"))
            .expect("load should succeed");

        let b = store
            .load_for_message(&message_id("msg-b"))
            .expect("load should succeed");

        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].message_id().as_str(), "msg-a");
        assert_eq!(b[0].message_id().as_str(), "msg-b");
    }

    #[test]
    fn distinct_evidence_for_same_message_remains_distinct() {
        let mut store = InMemoryDeliveryEventStore::new();

        store
            .append(event(
                "failure-1",
                "msg-4",
                "relay-a",
                DeliveryEventKind::DeliveryFailed,
            ))
            .expect("append should succeed");

        store
            .append(event(
                "received-1",
                "msg-4",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ))
            .expect("append should succeed");

        let loaded = store
            .load_for_message(&message_id("msg-4"))
            .expect("load should succeed");

        assert_eq!(loaded.len(), 2);
    }
}
