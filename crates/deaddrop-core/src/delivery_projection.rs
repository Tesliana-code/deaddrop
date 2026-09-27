use std::collections::HashMap;
use std::fmt;

use deaddrop_protocol::{DeliveryEvent, DeliveryEventId, DeliveryEventKind, MessageId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryProjectionError {
    MessageMismatch {
        expected: MessageId,
        found: MessageId,
    },
    EventIdentityConflict {
        event_id: DeliveryEventId,
    },
}

impl fmt::Display for DeliveryProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageMismatch { expected, found } => write!(
                f,
                "delivery event belongs to message {found}, expected {expected}"
            ),
            Self::EventIdentityConflict { event_id } => write!(
                f,
                "delivery event id {event_id} was reused for different evidence"
            ),
        }
    }
}

impl std::error::Error for DeliveryProjectionError {}

/// Replay-safe derived delivery knowledge for one immutable message.
///
/// Append-only delivery evidence remains authoritative.
/// This structure is only a rebuildable projection of observed facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryProjection {
    message_id: MessageId,
    seen_events: HashMap<DeliveryEventId, DeliveryEvent>,
    transport_acceptances: usize,
    recipient_received: bool,
    recipient_verified: bool,
    recipient_acknowledged: bool,
    failure_observations: usize,
    expiry_observations: usize,
}

impl DeliveryProjection {
    pub fn new(message_id: MessageId) -> Self {
        Self {
            message_id,
            seen_events: HashMap::new(),
            transport_acceptances: 0,
            recipient_received: false,
            recipient_verified: false,
            recipient_acknowledged: false,
            failure_observations: 0,
            expiry_observations: 0,
        }
    }

    pub fn apply(&mut self, event: &DeliveryEvent) -> Result<(), DeliveryProjectionError> {
        if event.message_id() != &self.message_id {
            return Err(DeliveryProjectionError::MessageMismatch {
                expected: self.message_id.clone(),
                found: event.message_id().clone(),
            });
        }

        if let Some(existing) = self.seen_events.get(event.id()) {
            if existing == event {
                return Ok(());
            }

            return Err(DeliveryProjectionError::EventIdentityConflict {
                event_id: event.id().clone(),
            });
        }

        self.seen_events.insert(event.id().clone(), event.clone());

        match event.kind() {
            DeliveryEventKind::TransportAccepted => {
                self.transport_acceptances += 1;
            }
            DeliveryEventKind::RecipientReceived => {
                self.recipient_received = true;
            }
            DeliveryEventKind::RecipientVerified => {
                self.recipient_verified = true;
            }
            DeliveryEventKind::RecipientAcknowledged => {
                self.recipient_acknowledged = true;
            }
            DeliveryEventKind::DeliveryFailed => {
                self.failure_observations += 1;
            }
            DeliveryEventKind::DeliveryExpired => {
                self.expiry_observations += 1;
            }
        }

        Ok(())
    }

    pub fn from_events<'a>(
        message_id: MessageId,
        events: impl IntoIterator<Item = &'a DeliveryEvent>,
    ) -> Result<Self, DeliveryProjectionError> {
        let mut projection = Self::new(message_id);

        for event in events {
            projection.apply(event)?;
        }

        Ok(projection)
    }

    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    pub fn transport_acceptances(&self) -> usize {
        self.transport_acceptances
    }

    pub fn recipient_received(&self) -> bool {
        self.recipient_received
    }

    pub fn recipient_verified(&self) -> bool {
        self.recipient_verified
    }

    pub fn recipient_acknowledged(&self) -> bool {
        self.recipient_acknowledged
    }

    pub fn failure_observations(&self) -> usize {
        self.failure_observations
    }

    pub fn expiry_observations(&self) -> usize {
        self.expiry_observations
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deaddrop_protocol::{DeliveryEventId, NodeId};

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
    fn projects_successful_delivery_evidence() {
        let events = [
            event(
                "e1",
                "msg-1",
                "relay-a",
                DeliveryEventKind::TransportAccepted,
            ),
            event(
                "e2",
                "msg-1",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ),
            event(
                "e3",
                "msg-1",
                "node-b",
                DeliveryEventKind::RecipientVerified,
            ),
            event(
                "e4",
                "msg-1",
                "node-b",
                DeliveryEventKind::RecipientAcknowledged,
            ),
        ];

        let projection = DeliveryProjection::from_events(message_id("msg-1"), &events)
            .expect("projection should succeed");

        assert_eq!(projection.transport_acceptances(), 1);
        assert!(projection.recipient_received());
        assert!(projection.recipient_verified());
        assert!(projection.recipient_acknowledged());
        assert_eq!(projection.failure_observations(), 0);
    }

    #[test]
    fn preserves_failure_evidence_when_later_delivery_succeeds() {
        let events = [
            event("e1", "msg-2", "relay-a", DeliveryEventKind::DeliveryFailed),
            event("e2", "msg-2", "relay-a", DeliveryEventKind::DeliveryFailed),
            event(
                "e3",
                "msg-2",
                "relay-b",
                DeliveryEventKind::TransportAccepted,
            ),
            event(
                "e4",
                "msg-2",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ),
        ];

        let projection = DeliveryProjection::from_events(message_id("msg-2"), &events)
            .expect("projection should succeed");

        assert_eq!(projection.failure_observations(), 2);
        assert_eq!(projection.transport_acceptances(), 1);
        assert!(projection.recipient_received());
    }

    #[test]
    fn exact_duplicate_event_replay_is_idempotent() {
        let received = event(
            "same-event",
            "msg-3",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        );

        let projection =
            DeliveryProjection::from_events(message_id("msg-3"), [&received, &received])
                .expect("exact replay should succeed");

        assert!(projection.recipient_received());
        assert_eq!(projection.seen_events.len(), 1);
    }

    #[test]
    fn duplicate_failure_replay_does_not_inflate_observations() {
        let failure = event(
            "failure-1",
            "msg-4",
            "relay-a",
            DeliveryEventKind::DeliveryFailed,
        );

        let projection = DeliveryProjection::from_events(message_id("msg-4"), [&failure, &failure])
            .expect("exact replay should succeed");

        assert_eq!(projection.failure_observations(), 1);
    }

    #[test]
    fn same_event_id_with_different_evidence_fails_closed() {
        let first = event(
            "collision-1",
            "msg-5",
            "relay-a",
            DeliveryEventKind::DeliveryFailed,
        );

        let conflicting = event(
            "collision-1",
            "msg-5",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        );

        let error = DeliveryProjection::from_events(message_id("msg-5"), [&first, &conflicting])
            .expect_err("conflicting event identity must fail");

        assert_eq!(
            error,
            DeliveryProjectionError::EventIdentityConflict {
                event_id: event_id("collision-1"),
            }
        );
    }

    #[test]
    fn expiry_does_not_erase_later_evidence() {
        let events = [
            event("e1", "msg-6", "relay-a", DeliveryEventKind::DeliveryExpired),
            event(
                "e2",
                "msg-6",
                "relay-b",
                DeliveryEventKind::TransportAccepted,
            ),
            event(
                "e3",
                "msg-6",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ),
        ];

        let projection = DeliveryProjection::from_events(message_id("msg-6"), &events)
            .expect("projection should succeed");

        assert_eq!(projection.expiry_observations(), 1);
        assert!(projection.recipient_received());
    }

    #[test]
    fn rejects_events_from_another_message() {
        let foreign = event(
            "foreign-event",
            "msg-other",
            "relay-a",
            DeliveryEventKind::TransportAccepted,
        );

        let error = DeliveryProjection::from_events(message_id("msg-7"), [&foreign])
            .expect_err("foreign message must be rejected");

        assert_eq!(
            error,
            DeliveryProjectionError::MessageMismatch {
                expected: message_id("msg-7"),
                found: message_id("msg-other"),
            }
        );
    }
}
