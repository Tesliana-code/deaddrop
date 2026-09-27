use crate::{MessageId, NodeId};

/// Append-only evidence about what happened to a message during delivery.
///
/// Delivery events do not mutate the message envelope and do not become
/// application-level success merely because transport succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryEventKind {
    /// A transport accepted responsibility for attempting delivery.
    TransportAccepted,

    /// The intended recipient node accepted the message bytes/envelope.
    RecipientReceived,

    /// The recipient successfully verified protocol integrity/authenticity.
    RecipientVerified,

    /// The recipient explicitly acknowledged the message at protocol level.
    ///
    /// This does not imply that a requested task or external action succeeded.
    RecipientAcknowledged,

    /// A delivery attempt failed.
    ///
    /// This is evidence about an attempt, not necessarily a terminal state;
    /// another attempt may occur later.
    DeliveryFailed,

    /// Delivery was abandoned because its permitted delivery window expired.
    DeliveryExpired,
}

impl DeliveryEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TransportAccepted => "transport_accepted",
            Self::RecipientReceived => "recipient_received",
            Self::RecipientVerified => "recipient_verified",
            Self::RecipientAcknowledged => "recipient_acknowledged",
            Self::DeliveryFailed => "delivery_failed",
            Self::DeliveryExpired => "delivery_expired",
        }
    }
}

/// A statement by one protocol participant about a delivery observation.
///
/// Ordering, persistence sequence, timestamps, and signatures are deliberately
/// not frozen in V0 yet. Those belong to later protocol/storage decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEvent {
    message_id: MessageId,
    reported_by: NodeId,
    kind: DeliveryEventKind,
}

impl DeliveryEvent {
    pub fn new(message_id: MessageId, reported_by: NodeId, kind: DeliveryEventKind) -> Self {
        Self {
            message_id,
            reported_by,
            kind,
        }
    }

    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    pub fn reported_by(&self) -> &NodeId {
        &self.reported_by
    }

    pub fn kind(&self) -> DeliveryEventKind {
        self.kind
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message_id(value: &str) -> MessageId {
        MessageId::parse(value).expect("valid message id")
    }

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("valid node id")
    }

    #[test]
    fn transport_acceptance_is_explicit_evidence() {
        let event = DeliveryEvent::new(
            message_id("msg-1"),
            node("relay-a"),
            DeliveryEventKind::TransportAccepted,
        );

        assert_eq!(event.message_id().as_str(), "msg-1");
        assert_eq!(event.reported_by().as_str(), "relay-a");
        assert_eq!(event.kind(), DeliveryEventKind::TransportAccepted);
    }

    #[test]
    fn recipient_ack_is_distinct_from_transport_delivery() {
        assert_ne!(
            DeliveryEventKind::RecipientAcknowledged,
            DeliveryEventKind::RecipientReceived
        );
    }

    #[test]
    fn failure_is_an_event_not_a_terminal_message_mutation() {
        let failed = DeliveryEvent::new(
            message_id("msg-2"),
            node("relay-a"),
            DeliveryEventKind::DeliveryFailed,
        );

        let later_received = DeliveryEvent::new(
            message_id("msg-2"),
            node("node-b"),
            DeliveryEventKind::RecipientReceived,
        );

        assert_eq!(failed.message_id(), later_received.message_id());
        assert_eq!(failed.kind(), DeliveryEventKind::DeliveryFailed);
        assert_eq!(later_received.kind(), DeliveryEventKind::RecipientReceived);
    }

    #[test]
    fn event_names_are_stable_protocol_vocabulary() {
        assert_eq!(
            DeliveryEventKind::TransportAccepted.as_str(),
            "transport_accepted"
        );
        assert_eq!(
            DeliveryEventKind::RecipientAcknowledged.as_str(),
            "recipient_acknowledged"
        );
        assert_eq!(
            DeliveryEventKind::DeliveryExpired.as_str(),
            "delivery_expired"
        );
    }
}
