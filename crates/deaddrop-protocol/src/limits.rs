use std::fmt;

/// Maximum length of one canonical EnvelopeV0 wire encoding, in bytes (R8).
///
/// This also bounds `body`: there is no separate body limit.
pub const MAX_CANONICAL_ENVELOPE_BYTES: usize = 262_144;

/// Maximum UTF-8 byte length of every opaque identifier (R8).
///
/// Applies to `MessageId`, `NodeId`, `CorrelationId` and `DeliveryEventId`.
/// Counted in bytes, not scalars, characters or UTF-16 units.
pub const MAX_IDENTIFIER_UTF8_BYTES: usize = 256;

/// Maximum number of entries in `artifact_refs` (R8). Duplicates count
/// individually.
pub const MAX_ARTIFACT_REFS: usize = 256;

/// An EnvelopeV0 that exceeds an R8 limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeLimitError {
    EnvelopeTooLarge { len: usize },
    TooManyArtifactRefs { count: usize },
}

impl fmt::Display for EnvelopeLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EnvelopeTooLarge { len } => write!(
                f,
                "canonical envelope is {len} bytes; the V0 maximum is {MAX_CANONICAL_ENVELOPE_BYTES}"
            ),
            Self::TooManyArtifactRefs { count } => write!(
                f,
                "envelope has {count} artifact refs; the V0 maximum is {MAX_ARTIFACT_REFS}"
            ),
        }
    }
}

impl std::error::Error for EnvelopeLimitError {}
