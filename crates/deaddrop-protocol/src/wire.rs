use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, PROTOCOL_V0};

#[derive(Debug)]
pub enum WireEnvelopeError {
    Json(serde_json::Error),
    UnsupportedProtocol { value: String },
    InvalidField { field: &'static str, value: String },
    UnknownMessageKind { value: String },
    NonCanonicalEncoding,
}

impl fmt::Display for WireEnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "wire JSON error: {error}"),
            Self::UnsupportedProtocol { value } => {
                write!(f, "unsupported wire protocol: {value:?}")
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
        }
    }
}

impl std::error::Error for WireEnvelopeError {}

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
pub fn encode_envelope_v0(envelope: &EnvelopeV0) -> Result<Vec<u8>, WireEnvelopeError> {
    let wire = WireEnvelopeV0::from(envelope);
    Ok(serde_json::to_vec(&wire)?)
}

/// Decode canonical EnvelopeV0 wire bytes.
///
/// The input must both parse as the frozen V0 schema and exactly equal the
/// canonical bytes obtained by re-encoding the reconstructed envelope.
pub fn decode_envelope_v0(bytes: &[u8]) -> Result<EnvelopeV0, WireEnvelopeError> {
    let wire: WireEnvelopeV0 = serde_json::from_slice(bytes)?;

    if wire.protocol != PROTOCOL_V0 {
        return Err(WireEnvelopeError::UnsupportedProtocol {
            value: wire.protocol,
        });
    }

    let id = MessageId::parse(wire.id.clone()).map_err(|_| WireEnvelopeError::InvalidField {
        field: "id",
        value: wire.id.clone(),
    })?;

    let from = NodeId::parse(wire.from.clone()).map_err(|_| WireEnvelopeError::InvalidField {
        field: "from",
        value: wire.from.clone(),
    })?;

    let to = NodeId::parse(wire.to.clone()).map_err(|_| WireEnvelopeError::InvalidField {
        field: "to",
        value: wire.to.clone(),
    })?;

    let kind =
        MessageKind::parse(&wire.kind).ok_or_else(|| WireEnvelopeError::UnknownMessageKind {
            value: wire.kind.clone(),
        })?;

    let correlation_id = wire
        .correlation_id
        .map(|value| {
            CorrelationId::parse(value.clone()).map_err(|_| WireEnvelopeError::InvalidField {
                field: "correlation_id",
                value,
            })
        })
        .transpose()?;

    let mut artifact_refs = Vec::with_capacity(wire.artifact_refs.len());

    for value in wire.artifact_refs {
        let artifact_ref =
            ArtifactRef::from_str(&value).map_err(|_| WireEnvelopeError::InvalidField {
                field: "artifact_refs",
                value,
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
