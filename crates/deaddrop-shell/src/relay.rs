//! The relay seam. A relay moves canonical envelopes, detached signature
//! records, and artifact bytes. It interprets nothing and verifies nothing.
//! V0 uses a mailbox relay; other transports can implement [`Relay`] later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use deaddrop_protocol::{
    ArtifactRef, EnvelopeV0, MessageId, NodeId, SignatureV0, decode_envelope_v0,
    decode_signature_v0, encode_envelope_v0, encode_signature_v0,
};
use deaddrop_store::{InMemoryMessageStore, InMemoryMessageStoreError, MessageStore};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum RelayError {
    /// The message id already identifies a different envelope.
    IdentityConflict {
        message_id: MessageId,
    },
    Failed(String),
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdentityConflict { message_id } => {
                write!(f, "relay already holds a different message {message_id}")
            }
            Self::Failed(detail) => write!(f, "relay failed: {detail}"),
        }
    }
}

impl std::error::Error for RelayError {}

pub trait Relay {
    /// Exact replay is success.
    fn post_message(&self, envelope: &EnvelopeV0) -> Result<(), RelayError>;
    /// Exact replay is success; distinct records for one id accumulate.
    fn post_signature(&self, record: &SignatureV0) -> Result<(), RelayError>;
    /// Ids addressed to `recipient`, in enumeration order only.
    fn list_for(&self, recipient: &NodeId) -> Result<Vec<MessageId>, RelayError>;
    fn get_message(&self, id: &MessageId) -> Result<Option<EnvelopeV0>, RelayError>;
    fn signatures(&self, id: &MessageId) -> Result<Vec<SignatureV0>, RelayError>;
    fn put_artifact(&self, bytes: &[u8]) -> Result<ArtifactRef, RelayError>;
    fn get_artifact(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError>;
}

impl<T: Relay + ?Sized> Relay for &T {
    fn post_message(&self, envelope: &EnvelopeV0) -> Result<(), RelayError> {
        (**self).post_message(envelope)
    }
    fn post_signature(&self, record: &SignatureV0) -> Result<(), RelayError> {
        (**self).post_signature(record)
    }
    fn list_for(&self, recipient: &NodeId) -> Result<Vec<MessageId>, RelayError> {
        (**self).list_for(recipient)
    }
    fn get_message(&self, id: &MessageId) -> Result<Option<EnvelopeV0>, RelayError> {
        (**self).get_message(id)
    }
    fn signatures(&self, id: &MessageId) -> Result<Vec<SignatureV0>, RelayError> {
        (**self).signatures(id)
    }
    fn put_artifact(&self, bytes: &[u8]) -> Result<ArtifactRef, RelayError> {
        (**self).put_artifact(bytes)
    }
    fn get_artifact(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError> {
        (**self).get_artifact(artifact)
    }
}

/// In-process relay with the same storage semantics as the node: Deaddrop's
/// own message store for envelopes, accumulating signature records, and
/// content-addressed artifacts.
#[derive(Default)]
pub struct MemoryRelay {
    messages: RefCell<InMemoryMessageStore>,
    signatures: RefCell<Vec<SignatureV0>>,
    artifacts: RefCell<HashMap<ArtifactRef, Vec<u8>>>,
}

impl Relay for MemoryRelay {
    fn post_message(&self, envelope: &EnvelopeV0) -> Result<(), RelayError> {
        match self.messages.borrow_mut().store(envelope.clone()) {
            Ok(_) => Ok(()),
            Err(InMemoryMessageStoreError::MessageIdentityConflict { message_id }) => {
                Err(RelayError::IdentityConflict { message_id })
            }
        }
    }

    fn post_signature(&self, record: &SignatureV0) -> Result<(), RelayError> {
        let mut signatures = self.signatures.borrow_mut();
        if !signatures.contains(record) {
            signatures.push(record.clone());
        }
        Ok(())
    }

    fn list_for(&self, recipient: &NodeId) -> Result<Vec<MessageId>, RelayError> {
        let ids = self.messages.borrow().ids_for_recipient(recipient);
        ids.map_err(|error| RelayError::Failed(error.to_string()))
    }

    fn get_message(&self, id: &MessageId) -> Result<Option<EnvelopeV0>, RelayError> {
        let envelope = self.messages.borrow().load(id);
        envelope.map_err(|error| RelayError::Failed(error.to_string()))
    }

    fn signatures(&self, id: &MessageId) -> Result<Vec<SignatureV0>, RelayError> {
        Ok(self
            .signatures
            .borrow()
            .iter()
            .filter(|record| record.message_id() == id)
            .cloned()
            .collect())
    }

    fn put_artifact(&self, bytes: &[u8]) -> Result<ArtifactRef, RelayError> {
        let artifact = ArtifactRef::from_sha256(Sha256::digest(bytes).into());
        self.artifacts
            .borrow_mut()
            .entry(artifact.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(artifact)
    }

    fn get_artifact(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError> {
        Ok(self.artifacts.borrow().get(artifact).cloned())
    }
}

/// Blocking client for the local `deaddrop-node` mailbox HTTP API.
pub struct HttpRelay {
    base_url: String,
    agent: ureq::Agent,
}

/// Matches the node's local artifact upload limit with headroom.
const READ_LIMIT_BYTES: u64 = 32 * 1024 * 1024;

impl HttpRelay {
    /// `base_url` like `http://127.0.0.1:8787`, without a trailing slash.
    pub fn new(base_url: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            agent,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }
}

type HttpResponse = ureq::http::Response<ureq::Body>;

fn failed(context: &str, detail: impl fmt::Display) -> RelayError {
    RelayError::Failed(format!("{context}: {detail}"))
}

fn read(context: &str, response: &mut HttpResponse) -> Result<(u16, Vec<u8>), RelayError> {
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(READ_LIMIT_BYTES)
        .read_to_vec()
        .map_err(|error| failed(context, error))?;
    Ok((status, body))
}

fn unexpected(context: &str, status: u16, body: &[u8]) -> RelayError {
    failed(
        context,
        format!("HTTP {status}: {}", String::from_utf8_lossy(body)),
    )
}

fn json(context: &str, body: &[u8]) -> Result<serde_json::Value, RelayError> {
    serde_json::from_slice(body).map_err(|error| failed(context, error))
}

impl Relay for HttpRelay {
    fn post_message(&self, envelope: &EnvelopeV0) -> Result<(), RelayError> {
        const CONTEXT: &str = "POST /v0/messages";
        let bytes = encode_envelope_v0(envelope).map_err(|error| failed(CONTEXT, error))?;
        let mut response = self
            .agent
            .post(self.url("/v0/messages"))
            .header("Content-Type", "application/json")
            .send(&bytes[..])
            .map_err(|error| failed(CONTEXT, error))?;
        match read(CONTEXT, &mut response)? {
            (200 | 201, _) => Ok(()),
            (409, _) => Err(RelayError::IdentityConflict {
                message_id: envelope.id().clone(),
            }),
            (status, body) => Err(unexpected(CONTEXT, status, &body)),
        }
    }

    fn post_signature(&self, record: &SignatureV0) -> Result<(), RelayError> {
        const CONTEXT: &str = "POST /v0/messages/{id}/signatures";
        let bytes = encode_signature_v0(record).map_err(|error| failed(CONTEXT, error))?;
        let mut response = self
            .agent
            .post(self.url(&format!(
                "/v0/messages/{}/signatures",
                percent_encode(record.message_id().as_str())
            )))
            .header("Content-Type", "application/json")
            .send(&bytes[..])
            .map_err(|error| failed(CONTEXT, error))?;
        match read(CONTEXT, &mut response)? {
            (200 | 201, _) => Ok(()),
            (status, body) => Err(unexpected(CONTEXT, status, &body)),
        }
    }

    fn list_for(&self, recipient: &NodeId) -> Result<Vec<MessageId>, RelayError> {
        const CONTEXT: &str = "GET /v0/messages?to=";
        let mut response = self
            .agent
            .get(self.url(&format!(
                "/v0/messages?to={}",
                percent_encode(recipient.as_str())
            )))
            .call()
            .map_err(|error| failed(CONTEXT, error))?;
        let (status, body) = read(CONTEXT, &mut response)?;
        if status != 200 {
            return Err(unexpected(CONTEXT, status, &body));
        }
        json(CONTEXT, &body)?["message_ids"]
            .as_array()
            .ok_or_else(|| failed(CONTEXT, "missing message_ids"))?
            .iter()
            .map(|id| {
                id.as_str()
                    .and_then(|id| MessageId::parse(id).ok())
                    .ok_or_else(|| failed(CONTEXT, "invalid message id"))
            })
            .collect()
    }

    fn get_message(&self, id: &MessageId) -> Result<Option<EnvelopeV0>, RelayError> {
        const CONTEXT: &str = "GET /v0/messages/{id}";
        let mut response = self
            .agent
            .get(self.url(&format!("/v0/messages/{}", percent_encode(id.as_str()))))
            .call()
            .map_err(|error| failed(CONTEXT, error))?;
        match read(CONTEXT, &mut response)? {
            (200, body) => decode_envelope_v0(&body)
                .map(Some)
                .map_err(|error| failed(CONTEXT, error)),
            (404, _) => Ok(None),
            (status, body) => Err(unexpected(CONTEXT, status, &body)),
        }
    }

    fn signatures(&self, id: &MessageId) -> Result<Vec<SignatureV0>, RelayError> {
        const CONTEXT: &str = "GET /v0/messages/{id}/signatures";
        let mut response = self
            .agent
            .get(self.url(&format!(
                "/v0/messages/{}/signatures",
                percent_encode(id.as_str())
            )))
            .call()
            .map_err(|error| failed(CONTEXT, error))?;
        let (status, body) = read(CONTEXT, &mut response)?;
        if status != 200 {
            return Err(unexpected(CONTEXT, status, &body));
        }
        json(CONTEXT, &body)?["signatures"]
            .as_array()
            .ok_or_else(|| failed(CONTEXT, "missing signatures"))?
            .iter()
            .map(|record| {
                let record = record
                    .as_str()
                    .ok_or_else(|| failed(CONTEXT, "signature is not a string"))?;
                decode_signature_v0(record.as_bytes()).map_err(|error| failed(CONTEXT, error))
            })
            .collect()
    }

    fn put_artifact(&self, bytes: &[u8]) -> Result<ArtifactRef, RelayError> {
        const CONTEXT: &str = "POST /v0/artifacts";
        let mut response = self
            .agent
            .post(self.url("/v0/artifacts"))
            .header("Content-Type", "application/octet-stream")
            .send(bytes)
            .map_err(|error| failed(CONTEXT, error))?;
        match read(CONTEXT, &mut response)? {
            (200 | 201, body) => json(CONTEXT, &body)?["artifact_ref"]
                .as_str()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| failed(CONTEXT, "invalid artifact_ref")),
            (status, body) => Err(unexpected(CONTEXT, status, &body)),
        }
    }

    fn get_artifact(&self, artifact: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError> {
        const CONTEXT: &str = "GET /v0/artifacts/{ref}";
        let mut response = self
            .agent
            .get(self.url(&format!(
                "/v0/artifacts/{}",
                percent_encode(&artifact.to_string())
            )))
            .call()
            .map_err(|error| failed(CONTEXT, error))?;
        match read(CONTEXT, &mut response)? {
            (200, body) => Ok(Some(body)),
            (404, _) => Ok(None),
            (status, body) => Err(unexpected(CONTEXT, status, &body)),
        }
    }
}

/// Encode everything except RFC 3986 unreserved characters.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
