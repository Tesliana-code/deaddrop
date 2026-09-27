use crate::{DeliveryEventId, MessageId, NodeId};

/// Append-only evidence about what happened to a message during delivery.
///
/// Delivery events do not mutate the message envelope and do not become
/// application-level success merely because transport succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryEventKind {
    TransportAccepted,
    RecipientReceived,
    RecipientVerified,

    /// Protocol-level acknowledgment only.
    ///
    /// This does not imply that a requested task or external action succeeded.
    RecipientAcknowledged,

    /// Evidence that a delivery operation failed.
    ///
    /// This does not imply permanent failure; later evidence may show success.
    DeliveryFailed,

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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "transport_accepted" => Some(Self::TransportAccepted),
            "recipient_received" => Some(Self::RecipientReceived),
            "recipient_verified" => Some(Self::RecipientVerified),
            "recipient_acknowledged" => Some(Self::RecipientAcknowledged),
            "delivery_failed" => Some(Self::DeliveryFailed),
            "delivery_expired" => Some(Self::DeliveryExpired),
            _ => None,
        }
    }
}

/// One uniquely identifiable delivery observation.
///
/// Event identity makes projection replay idempotent without requiring
/// timestamps or global ordering to be frozen in V0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEvent {
    id: DeliveryEventId,
    message_id: MessageId,
    reported_by: NodeId,
    kind: DeliveryEventKind,
}

impl DeliveryEvent {
    pub fn new(
        id: DeliveryEventId,
        message_id: MessageId,
        reported_by: NodeId,
        kind: DeliveryEventKind,
    ) -> Self {
        Self {
            id,
            message_id,
            reported_by,
            kind,
        }
    }

    pub fn id(&self) -> &DeliveryEventId {
        &self.id
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

    fn event_id(value: &str) -> DeliveryEventId {
        DeliveryEventId::parse(value).expect("valid delivery event id")
    }

    fn message_id(value: &str) -> MessageId {
        MessageId::parse(value).expect("valid message id")
    }

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("valid node id")
    }

    #[test]
    fn transport_acceptance_is_explicit_evidence() {
        let event = DeliveryEvent::new(
            event_id("delivery-1"),
            message_id("msg-1"),
            node("relay-a"),
            DeliveryEventKind::TransportAccepted,
        );

        assert_eq!(event.id().as_str(), "delivery-1");
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
    fn failure_is_evidence_not_terminal_message_mutation() {
        let failed = DeliveryEvent::new(
            event_id("delivery-2"),
            message_id("msg-2"),
            node("relay-a"),
            DeliveryEventKind::DeliveryFailed,
        );

        let later_received = DeliveryEvent::new(
            event_id("delivery-3"),
            message_id("msg-2"),
            node("node-b"),
            DeliveryEventKind::RecipientReceived,
        );

        assert_eq!(failed.message_id(), later_received.message_id());
        assert_ne!(failed.id(), later_received.id());
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
