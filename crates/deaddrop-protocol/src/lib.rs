mod artifact;
mod delivery;
mod ids;
mod limits;
mod message;
mod wire;

pub use artifact::{ArtifactRef, ArtifactRefError};
pub use delivery::{DeliveryEvent, DeliveryEventKind};
pub use ids::{CorrelationId, DeliveryEventId, IdError, MessageId, NodeId};
pub use limits::{
    EnvelopeLimitError, MAX_ARTIFACT_REFS, MAX_CANONICAL_ENVELOPE_BYTES, MAX_IDENTIFIER_UTF8_BYTES,
};
pub use message::{EnvelopeV0, MessageKind, PROTOCOL_V0};

pub use wire::{
    WireEnvelopeError, check_envelope_v0_limits, decode_envelope_v0, encode_envelope_v0,
};
