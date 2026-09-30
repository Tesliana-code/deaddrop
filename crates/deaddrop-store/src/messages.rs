use std::collections::HashMap;
use std::fmt;

use deaddrop_protocol::{
    EnvelopeLimitError, EnvelopeV0, MessageId, NodeId, check_envelope_v0_limits,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStoreOutcome {
    Stored,
    AlreadyPresent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InMemoryMessageStoreError {
    MessageIdentityConflict { message_id: MessageId },
    EnvelopeLimit(EnvelopeLimitError),
}

impl fmt::Display for InMemoryMessageStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageIdentityConflict { message_id } => write!(
                f,
                "message id {message_id} already identifies a different envelope"
            ),
            Self::EnvelopeLimit(error) => write!(f, "envelope exceeds V0 limits: {error}"),
        }
    }
}

impl std::error::Error for InMemoryMessageStoreError {}

/// Persistence boundary for immutable semantic envelopes.
///
/// A message id identifies exactly one EnvelopeV0. Replaying that exact
/// envelope is idempotent. Reusing the same id for different content fails
/// closed. An envelope that exceeds the V0 protocol limits is refused,
/// however it was constructed.
pub trait MessageStore {
    type Error;

    fn store(&mut self, envelope: EnvelopeV0) -> Result<MessageStoreOutcome, Self::Error>;

    fn load(&self, message_id: &MessageId) -> Result<Option<EnvelopeV0>, Self::Error>;

    /// Ids of the stored envelopes whose `to` is exactly `recipient`.
    ///
    /// Read-only. Each id appears once, sorted ascending by `MessageId`
    /// (byte-wise UTF-8). That order is for deterministic enumeration only:
    /// it is not semantic, causal, chronological, delivery, or priority order.
    fn ids_for_recipient(&self, recipient: &NodeId) -> Result<Vec<MessageId>, Self::Error>;
}

#[derive(Debug, Default)]
pub struct InMemoryMessageStore {
    envelopes: HashMap<MessageId, EnvelopeV0>,
}

impl InMemoryMessageStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MessageStore for InMemoryMessageStore {
    type Error = InMemoryMessageStoreError;

    fn store(&mut self, envelope: EnvelopeV0) -> Result<MessageStoreOutcome, Self::Error> {
        check_envelope_v0_limits(&envelope).map_err(InMemoryMessageStoreError::EnvelopeLimit)?;

        if let Some(existing) = self.envelopes.get(envelope.id()) {
            if existing == &envelope {
                return Ok(MessageStoreOutcome::AlreadyPresent);
            }

            return Err(InMemoryMessageStoreError::MessageIdentityConflict {
                message_id: envelope.id().clone(),
            });
        }

        self.envelopes.insert(envelope.id().clone(), envelope);

        Ok(MessageStoreOutcome::Stored)
    }

    fn load(&self, message_id: &MessageId) -> Result<Option<EnvelopeV0>, Self::Error> {
        Ok(self.envelopes.get(message_id).cloned())
    }

    fn ids_for_recipient(&self, recipient: &NodeId) -> Result<Vec<MessageId>, Self::Error> {
        let mut ids: Vec<MessageId> = self
            .envelopes
            .values()
            .filter(|envelope| envelope.to() == recipient)
            .map(|envelope| envelope.id().clone())
            .collect();

        ids.sort();

        Ok(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deaddrop_protocol::MessageKind;

    fn envelope(id: &str, from: &str, to: &str, body: &str) -> EnvelopeV0 {
        EnvelopeV0::new(
            MessageId::parse(id).unwrap(),
            NodeId::parse(from).unwrap(),
            NodeId::parse(to).unwrap(),
            MessageKind::Message,
            body,
        )
    }

    #[test]
    fn stores_and_loads_exact_envelope() {
        let mut store = InMemoryMessageStore::new();

        let message = envelope("msg-1", "node-a", "node-b", "hello");

        assert_eq!(
            store.store(message.clone()).unwrap(),
            MessageStoreOutcome::Stored
        );

        assert_eq!(
            store.load(&MessageId::parse("msg-1").unwrap()).unwrap(),
            Some(message)
        );
    }

    #[test]
    fn exact_replay_is_idempotent() {
        let mut store = InMemoryMessageStore::new();

        let message = envelope("msg-2", "node-a", "node-b", "same");

        assert_eq!(
            store.store(message.clone()).unwrap(),
            MessageStoreOutcome::Stored
        );

        assert_eq!(
            store.store(message).unwrap(),
            MessageStoreOutcome::AlreadyPresent
        );
    }

    #[test]
    fn conflicting_message_identity_fails_closed() {
        let mut store = InMemoryMessageStore::new();

        store
            .store(envelope("collision", "node-a", "node-b", "first"))
            .unwrap();

        let result = store.store(envelope("collision", "node-a", "node-b", "different"));

        assert!(matches!(
            result,
            Err(InMemoryMessageStoreError::MessageIdentityConflict { .. })
        ));
    }
}
