mod artifact;
mod delivery;
mod ids;
mod message;
mod wire;

pub use artifact::{ArtifactRef, ArtifactRefError};
pub use delivery::{DeliveryEvent, DeliveryEventKind};
pub use ids::{CorrelationId, DeliveryEventId, IdError, MessageId, NodeId};
pub use message::{EnvelopeV0, MessageKind, PROTOCOL_V0};

pub use wire::{WireEnvelopeError, decode_envelope_v0, encode_envelope_v0};
