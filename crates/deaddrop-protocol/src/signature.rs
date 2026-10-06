//! Detached signature records (`deaddrop-sig/0`).
//!
//! A SignatureV0 names a message, the node claiming to have signed it, the
//! Ed25519 public key used, and the signature bytes. It is data only: this
//! crate freezes the canonical wire shape and field formats and performs no
//! cryptography. What is signed (the exact canonical EnvelopeV0 bytes) and
//! whether a record is trusted are decided by the verifying node.
//!
//! EnvelopeV0 is unchanged; signatures travel beside it, never inside it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{MessageId, NodeId};

pub const SIGNATURE_PROTOCOL_V0: &str = "deaddrop-sig/0";

const KEY_PREFIX: &str = "ed25519:";
const KEY_BYTES: usize = 32;
const SIGNATURE_BYTES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ed25519PublicKeyError {
    UnsupportedScheme,
    InvalidLength,
    InvalidHex,
}

impl fmt::Display for Ed25519PublicKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedScheme => write!(f, "public key must use the ed25519 scheme"),
            Self::InvalidLength => {
                write!(f, "ed25519 public key must be 64 hexadecimal characters")
            }
            Self::InvalidHex => {
                write!(f, "ed25519 public key must be lowercase hexadecimal")
            }
        }
    }
}

impl std::error::Error for Ed25519PublicKeyError {}

/// An Ed25519 public key in its text form `ed25519:<64 lowercase hex>`.
///
/// Format only: whether the bytes are a valid curve point is checked by the
/// verifier, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ed25519PublicKey([u8; KEY_BYTES]);

impl Ed25519PublicKey {
    pub fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

impl fmt::Display for Ed25519PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(KEY_PREFIX)?;
        write_hex(f, &self.0)
    }
}

impl FromStr for Ed25519PublicKey {
    type Err = Ed25519PublicKeyError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value
            .strip_prefix(KEY_PREFIX)
            .ok_or(Ed25519PublicKeyError::UnsupportedScheme)?;
        if hex.len() != KEY_BYTES * 2 {
            return Err(Ed25519PublicKeyError::InvalidLength);
        }
        decode_hex(hex)
            .map(Self)
            .ok_or(Ed25519PublicKeyError::InvalidHex)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureV0 {
    message_id: MessageId,
    signer: NodeId,
    key: Ed25519PublicKey,
    signature: [u8; SIGNATURE_BYTES],
}

impl SignatureV0 {
    pub fn new(
        message_id: MessageId,
        signer: NodeId,
        key: Ed25519PublicKey,
        signature: [u8; SIGNATURE_BYTES],
    ) -> Self {
        Self {
            message_id,
            signer,
            key,
            signature,
        }
    }

    pub fn protocol(&self) -> &'static str {
        SIGNATURE_PROTOCOL_V0
    }

    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }

    pub fn signer(&self) -> &NodeId {
        &self.signer
    }

    pub fn key(&self) -> &Ed25519PublicKey {
        &self.key
    }

    pub fn signature(&self) -> &[u8; SIGNATURE_BYTES] {
        &self.signature
    }
}

#[derive(Debug)]
pub enum WireSignatureError {
    Json(serde_json::Error),
    UnsupportedProtocol { value: String },
    InvalidField { field: &'static str, value: String },
    NonCanonicalEncoding,
}

impl fmt::Display for WireSignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "signature JSON error: {error}"),
            Self::UnsupportedProtocol { value } => {
                write!(f, "unsupported signature protocol: {value:?}")
            }
            Self::InvalidField { field, value } => {
                write!(f, "invalid signature field {field}: {value:?}")
            }
            Self::NonCanonicalEncoding => write!(
                f,
                "signature record is valid JSON but not canonical encoding"
            ),
        }
    }
}

impl std::error::Error for WireSignatureError {}

impl From<serde_json::Error> for WireSignatureError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Field order is frozen by this struct.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireSignatureV0 {
    protocol: String,
    message_id: String,
    signer: String,
    key: String,
    signature: String,
}

/// Produce the one canonical UTF-8 JSON representation of a SignatureV0.
pub fn encode_signature_v0(record: &SignatureV0) -> Result<Vec<u8>, WireSignatureError> {
    let mut signature = String::with_capacity(SIGNATURE_BYTES * 2);
    for byte in record.signature {
        signature.push_str(&format!("{byte:02x}"));
    }
    let wire = WireSignatureV0 {
        protocol: SIGNATURE_PROTOCOL_V0.to_owned(),
        message_id: record.message_id.as_str().to_owned(),
        signer: record.signer.as_str().to_owned(),
        key: record.key.to_string(),
        signature,
    };
    Ok(serde_json::to_vec(&wire)?)
}

/// Decode canonical SignatureV0 bytes. Like `decode_envelope_v0`, the input
/// must re-encode to exactly the same bytes.
pub fn decode_signature_v0(bytes: &[u8]) -> Result<SignatureV0, WireSignatureError> {
    let wire: WireSignatureV0 = serde_json::from_slice(bytes)?;
    if wire.protocol != SIGNATURE_PROTOCOL_V0 {
        return Err(WireSignatureError::UnsupportedProtocol {
            value: wire.protocol,
        });
    }
    let invalid = |field: &'static str, value: &str| WireSignatureError::InvalidField {
        field,
        value: value.to_owned(),
    };
    let message_id = MessageId::parse(wire.message_id.clone())
        .map_err(|_| invalid("message_id", &wire.message_id))?;
    let signer = NodeId::parse(wire.signer.clone()).map_err(|_| invalid("signer", &wire.signer))?;
    let key: Ed25519PublicKey = wire.key.parse().map_err(|_| invalid("key", &wire.key))?;
    let signature = (wire.signature.len() == SIGNATURE_BYTES * 2)
        .then(|| decode_hex(&wire.signature))
        .flatten()
        .ok_or_else(|| invalid("signature", &wire.signature))?;

    let record = SignatureV0::new(message_id, signer, key, signature);
    if encode_signature_v0(&record)? != bytes {
        return Err(WireSignatureError::NonCanonicalEncoding);
    }
    Ok(record)
}

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for byte in bytes {
        write!(f, "{byte:02x}")?;
    }
    Ok(())
}

/// Lowercase hexadecimal only, so every value has exactly one text form.
fn decode_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    let digits = hex.as_bytes();
    if digits.len() != N * 2 {
        return None;
    }
    let nibble = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; N];
    for (i, pair) in digits.chunks_exact(2).enumerate() {
        out[i] = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Some(out)
}
