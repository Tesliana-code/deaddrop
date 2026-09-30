use std::fmt;
use std::io;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{
    ArtifactRef, CorrelationId, EnvelopeLimitError, EnvelopeV0, IdError, MAX_ARTIFACT_REFS,
    MAX_CANONICAL_ENVELOPE_BYTES, MessageId, MessageKind, NodeId, PROTOCOL_V0,
};

/// Longest prefix of a rejected input value that an error carries.
///
/// Errors are rendered to callers (HTTP responses, CLI stderr); they must not
/// echo arbitrarily large input back.
const ERROR_VALUE_PREVIEW_BYTES: usize = 64;

/// Longest JSON parser message rendered; serde_json messages can quote input
/// (unknown member names, mistyped string values).
const ERROR_JSON_MESSAGE_BYTES: usize = 256;

/// Bounded, rendering-safe copy of a rejected value.
fn preview(value: &str) -> String {
    bounded(value, ERROR_VALUE_PREVIEW_BYTES)
}

fn bounded(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }

    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}... ({} bytes)", &value[..end], value.len())
}

#[derive(Debug)]
pub enum WireEnvelopeError {
    Json(serde_json::Error),
    UnsupportedProtocol {
        value: String,
    },
    InvalidIdentifier {
        field: &'static str,
        error: IdError,
        value: String,
    },
    InvalidField {
        field: &'static str,
        value: String,
    },
    UnknownMessageKind {
        value: String,
    },
    NonCanonicalEncoding,
    Limit(EnvelopeLimitError),
}

impl fmt::Display for WireEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(
                f,
                "wire JSON error: {}",
                bounded(&error.to_string(), ERROR_JSON_MESSAGE_BYTES)
            ),
            Self::UnsupportedProtocol { value } => {
                write!(f, "unsupported wire protocol: {value:?}")
            }
            Self::InvalidIdentifier {
                field,
                error,
                value,
            } => {
                write!(f, "invalid wire identifier {field}: {error}: {value:?}")
            }
            Self::InvalidField { field, value } => {
                write!(f, "invalid wire field {field}: {value:?}")
            }
            Self::UnknownMessageKind { value } => {
                write!(f, "unknown wire message kind: {value:?}")
            }
            Self::NonCanonicalEncoding => {
                write!(
                    f,
                    "wire envelope is valid JSON but not canonical V0 encoding"
                )
            }
            Self::Limit(error) => write!(f, "wire envelope limit exceeded: {error}"),
        }
    }
}

impl std::error::Error for WireEnvelopeError {}

impl From<EnvelopeLimitError> for WireEnvelopeError {
    fn from(error: EnvelopeLimitError) -> Self {
        Self::Limit(error)
    }
}

impl From<serde_json::Error> for WireEnvelopeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelopeV0 {
    protocol: String,
    id: String,
    from: String,
    to: String,
    kind: String,
    correlation_id: Option<String>,
    body: String,
    artifact_refs: Vec<String>,
}

impl From<&EnvelopeV0> for WireEnvelopeV0 {
    fn from(envelope: &EnvelopeV0) -> Self {
        Self {
            protocol: envelope.protocol().to_owned(),
            id: envelope.id().as_str().to_owned(),
            from: envelope.from().as_str().to_owned(),
            to: envelope.to().as_str().to_owned(),
            kind: envelope.kind().as_str().to_owned(),
            correlation_id: envelope
                .correlation_id()
                .map(|value| value.as_str().to_owned()),
            body: envelope.body().to_owned(),
            artifact_refs: envelope
                .artifact_refs()
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

/// Produce the one canonical UTF-8 JSON representation of an EnvelopeV0.
///
/// Field order is frozen by WireEnvelopeV0. Optional correlation identity is
/// represented explicitly as either a string or JSON null. Artifact order is
/// preserved.
///
/// Refuses (R7) an envelope that exceeds an R8 limit.
pub fn encode_envelope_v0(envelope: &EnvelopeV0) -> Result<Vec<u8>, WireEnvelopeError> {
    check_artifact_ref_count(envelope.artifact_refs().len())?;

    let bytes = serde_json::to_vec(&WireEnvelopeV0::from(envelope))?;
    check_envelope_len(bytes.len())?;

    Ok(bytes)
}

/// Check an in-memory EnvelopeV0 against the R8 limits without producing
/// its wire bytes.
///
/// Identifier limits are already guaranteed by the identifier types; this
/// covers the artifact ref count and the canonical encoded length.
pub fn check_envelope_v0_limits(envelope: &EnvelopeV0) -> Result<(), EnvelopeLimitError> {
    check_artifact_ref_count(envelope.artifact_refs().len())?;

    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, &WireEnvelopeV0::from(envelope))
        .expect("serializing strings into a counting sink cannot fail");

    check_envelope_len(counter.0)
}

fn check_artifact_ref_count(count: usize) -> Result<(), EnvelopeLimitError> {
    if count > MAX_ARTIFACT_REFS {
        return Err(EnvelopeLimitError::TooManyArtifactRefs { count });
    }

    Ok(())
}

fn check_envelope_len(len: usize) -> Result<(), EnvelopeLimitError> {
    if len > MAX_CANONICAL_ENVELOPE_BYTES {
        return Err(EnvelopeLimitError::EnvelopeTooLarge { len });
    }

    Ok(())
}

struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Decode canonical EnvelopeV0 wire bytes.
///
/// The input must both parse as the frozen V0 schema and exactly equal the
/// canonical bytes obtained by re-encoding the reconstructed envelope.
///
/// Input longer than the R8 envelope limit is rejected before JSON parsing.
pub fn decode_envelope_v0(bytes: &[u8]) -> Result<EnvelopeV0, WireEnvelopeError> {
    check_envelope_len(bytes.len())?;

    let wire: WireEnvelopeV0 = serde_json::from_slice(bytes)?;

    if wire.protocol != PROTOCOL_V0 {
        return Err(WireEnvelopeError::UnsupportedProtocol {
            value: preview(&wire.protocol),
        });
    }

    let id = MessageId::parse(wire.id.clone())
        .map_err(|error| invalid_identifier("id", error, &wire.id))?;

    let from = NodeId::parse(wire.from.clone())
        .map_err(|error| invalid_identifier("from", error, &wire.from))?;

    let to = NodeId::parse(wire.to.clone())
        .map_err(|error| invalid_identifier("to", error, &wire.to))?;

    let kind =
        MessageKind::parse(&wire.kind).ok_or_else(|| WireEnvelopeError::UnknownMessageKind {
            value: preview(&wire.kind),
        })?;

    let correlation_id = wire
        .correlation_id
        .map(|value| {
            CorrelationId::parse(value.clone())
                .map_err(|error| invalid_identifier("correlation_id", error, &value))
        })
        .transpose()?;

    check_artifact_ref_count(wire.artifact_refs.len())?;

    let mut artifact_refs = Vec::with_capacity(wire.artifact_refs.len());

    for value in wire.artifact_refs {
        let artifact_ref =
            ArtifactRef::from_str(&value).map_err(|_| WireEnvelopeError::InvalidField {
                field: "artifact_refs",
                value: preview(&value),
            })?;

        artifact_refs.push(artifact_ref);
    }

    let mut envelope = EnvelopeV0::new(id, from, to, kind, wire.body);

    if let Some(correlation_id) = correlation_id {
        envelope = envelope.with_correlation_id(correlation_id);
    }

    for artifact_ref in artifact_refs {
        envelope = envelope.with_artifact_ref(artifact_ref);
    }

    let canonical = encode_envelope_v0(&envelope)?;

    if canonical != bytes {
        return Err(WireEnvelopeError::NonCanonicalEncoding);
    }

    Ok(envelope)
}

fn invalid_identifier(field: &'static str, error: IdError, value: &str) -> WireEnvelopeError {
    WireEnvelopeError::InvalidIdentifier {
        field,
        error,
        value: preview(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_bounds_rejected_values_on_char_boundaries() {
        assert_eq!(preview("short"), "short");

        let long = format!("{}{}", "a".repeat(63), "\u{e9}".repeat(100));
        let bounded = preview(&long);
        assert!(bounded.starts_with(&"a".repeat(63)));
        assert!(bounded.ends_with("... (263 bytes)"));
        assert!(bounded.len() < 100);
    }
}
