//! Node-local shell operations.
//!
//! A node home directory holds everything the node owns:
//!
//! - `identity.json`: node id, public key, relay URL
//! - `secret.key`: Ed25519 secret seed, owner-only (0600)
//! - `peers.json`: explicitly trusted peers and their keys
//! - `messages.sqlite3`: envelopes this node sent or verified on receipt
//! - `delivery.sqlite3`: append-only delivery evidence
//! - `artifacts/`: content-addressed artifact bytes
//!
//! Stores are opened per operation, so independent processes can share a
//! home without holding long-lived handles.

use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, DeliveryEvent, DeliveryEventId, DeliveryEventKind,
    Ed25519PublicKey, EnvelopeV0, MessageId, MessageKind, NodeId,
};
use deaddrop_store::{
    ArtifactPutOutcome, ArtifactStore, DeliveryEventStore, FilesystemArtifactStore, MessageStore,
    SqliteDeliveryEventStore, SqliteMessageStore,
};
use serde::{Deserialize, Serialize};

use crate::key::LocalKey;
use crate::relay::{HttpRelay, Relay, RelayError};
use crate::verify::{Rejection, verify};

const IDENTITY: &str = "identity.json";
const SECRET: &str = "secret.key";
const PEERS: &str = "peers.json";
const MESSAGES: &str = "messages.sqlite3";
const DELIVERY: &str = "delivery.sqlite3";
const ARTIFACTS: &str = "artifacts";

#[derive(Debug)]
pub enum ShellError {
    AlreadyInitialized,
    NotInitialized,
    InvalidRelay {
        relay: String,
    },
    /// Slice 1 carries plaintext bodies; only a loopback relay is allowed
    /// until encryption exists.
    NonLoopbackRelay {
        relay: String,
    },
    PeerKeyConflict {
        node: NodeId,
    },
    UnknownPeer {
        node: NodeId,
    },
    NotAcknowledgeable {
        message_id: MessageId,
    },
    ArtifactNotFound {
        artifact: ArtifactRef,
    },
    ArtifactMismatch {
        artifact: ArtifactRef,
    },
    Relay(RelayError),
    Store(String),
    Io(std::io::Error),
    Corrupt(String),
    Randomness(String),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyInitialized => write!(f, "node home is already initialized"),
            Self::NotInitialized => write!(f, "node home is not initialized"),
            Self::InvalidRelay { relay } => write!(f, "invalid relay URL {relay:?}"),
            Self::NonLoopbackRelay { relay } => write!(
                f,
                "refusing non-loopback relay {relay:?}: Slice 1 bodies are not encrypted; \
                 encryption is required before a non-loopback relay is production-capable"
            ),
            Self::PeerKeyConflict { node } => {
                write!(f, "peer {node} is already trusted with a different key")
            }
            Self::UnknownPeer { node } => write!(f, "{node} is not a trusted peer"),
            Self::NotAcknowledgeable { message_id } => write!(
                f,
                "{message_id} is not a verified message received by this node"
            ),
            Self::ArtifactNotFound { artifact } => write!(f, "artifact {artifact} not found"),
            Self::ArtifactMismatch { artifact } => {
                write!(f, "artifact bytes do not match {artifact}")
            }
            Self::Relay(error) => write!(f, "{error}"),
            Self::Store(detail) => write!(f, "local store: {detail}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Corrupt(what) => write!(f, "corrupt node home: {what}"),
            Self::Randomness(detail) => write!(f, "OS randomness unavailable: {detail}"),
        }
    }
}

impl std::error::Error for ShellError {}

impl From<std::io::Error> for ShellError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<RelayError> for ShellError {
    fn from(error: RelayError) -> Self {
        Self::Relay(error)
    }
}

fn store_error(error: impl fmt::Display) -> ShellError {
    ShellError::Store(error.to_string())
}

/// What a node publishes about itself for out-of-band peer enrollment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityCard {
    pub node: NodeId,
    pub key: Ed25519PublicKey,
    pub relay: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityFile {
    node_id: String,
    key: String,
    relay: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PeersFile {
    peers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerOutcome {
    Added,
    AlreadyPresent,
}

/// A verified acknowledgment of a message this node sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledged {
    pub ack: MessageId,
    pub message: MessageId,
    pub by: NodeId,
}

/// What one `sync` pass found. Rejected messages are not stored and are
/// reconsidered on the next pass (for example after a peer is added).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub received: Vec<MessageId>,
    pub acknowledged: Vec<Acknowledged>,
    pub rejected: Vec<(MessageId, Rejection)>,
}

/// Create a node home with a fresh Ed25519 identity.
pub fn init(dir: &Path, node: NodeId, relay: &str) -> Result<IdentityCard, ShellError> {
    require_loopback(relay)?;
    if dir.join(IDENTITY).exists() || dir.join(SECRET).exists() {
        return Err(ShellError::AlreadyInitialized);
    }
    std::fs::create_dir_all(dir)?;
    let key = LocalKey::generate()?;
    key.save(&dir.join(SECRET))?;
    let card = IdentityCard {
        node,
        key: key.public(),
        relay: relay.to_owned(),
    };
    write_json(
        &dir.join(IDENTITY),
        &IdentityFile {
            node_id: card.node.to_string(),
            key: card.key.to_string(),
            relay: card.relay.clone(),
        },
    )?;
    Ok(card)
}

/// Accept only `http(s)://` URLs whose host is loopback.
fn require_loopback(relay: &str) -> Result<(), ShellError> {
    let invalid = || ShellError::InvalidRelay {
        relay: relay.to_owned(),
    };
    let rest = relay
        .strip_prefix("http://")
        .or_else(|| relay.strip_prefix("https://"))
        .ok_or_else(invalid)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(invalid());
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        bracketed.split(']').next().ok_or_else(invalid)?
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        return Err(invalid());
    }
    let loopback = host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    if loopback {
        Ok(())
    } else {
        Err(ShellError::NonLoopbackRelay {
            relay: relay.to_owned(),
        })
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), ShellError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(store_error)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub struct Shell<R> {
    dir: PathBuf,
    card: IdentityCard,
    key: LocalKey,
    relay: R,
}

impl Shell<HttpRelay> {
    /// Open a node home, reaching its configured relay over HTTP.
    pub fn open(dir: &Path) -> Result<Self, ShellError> {
        let relay = read_identity(dir)?.relay;
        Self::open_with(dir, HttpRelay::new(relay))
    }
}

fn read_identity(dir: &Path) -> Result<IdentityCard, ShellError> {
    let path = dir.join(IDENTITY);
    if !path.exists() {
        return Err(ShellError::NotInitialized);
    }
    let file: IdentityFile = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|_| ShellError::Corrupt(IDENTITY.into()))?;
    Ok(IdentityCard {
        node: NodeId::parse(file.node_id).map_err(|_| ShellError::Corrupt("node id".into()))?,
        key: file
            .key
            .parse()
            .map_err(|_| ShellError::Corrupt("public key".into()))?,
        relay: file.relay,
    })
}

impl<R: Relay> Shell<R> {
    pub fn open_with(dir: &Path, relay: R) -> Result<Self, ShellError> {
        let card = read_identity(dir)?;
        let key = LocalKey::load(&dir.join(SECRET))?;
        if key.public() != card.key {
            return Err(ShellError::Corrupt(
                "secret does not match public key".into(),
            ));
        }
        Ok(Self {
            dir: dir.to_owned(),
            card,
            key,
            relay,
        })
    }

    pub fn identity(&self) -> IdentityCard {
        self.card.clone()
    }

    fn node(&self) -> &NodeId {
        &self.card.node
    }

    fn messages(&self) -> Result<SqliteMessageStore, ShellError> {
        SqliteMessageStore::open(self.dir.join(MESSAGES)).map_err(store_error)
    }

    fn delivery_events(&self) -> Result<SqliteDeliveryEventStore, ShellError> {
        SqliteDeliveryEventStore::open(self.dir.join(DELIVERY)).map_err(store_error)
    }

    fn artifacts(&self) -> Result<FilesystemArtifactStore, ShellError> {
        FilesystemArtifactStore::open(self.dir.join(ARTIFACTS)).map_err(store_error)
    }

    fn load_peers(&self) -> Result<BTreeMap<NodeId, Ed25519PublicKey>, ShellError> {
        let path = self.dir.join(PEERS);
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let file: PeersFile = serde_json::from_slice(&std::fs::read(path)?)
            .map_err(|_| ShellError::Corrupt(PEERS.into()))?;
        file.peers
            .into_iter()
            .map(|(node, key)| {
                Ok((
                    NodeId::parse(node).map_err(|_| ShellError::Corrupt(PEERS.into()))?,
                    key.parse().map_err(|_| ShellError::Corrupt(PEERS.into()))?,
                ))
            })
            .collect()
    }

    /// Trust `key` to authenticate messages from `node`. Nothing else is
    /// granted. A different key for an already trusted peer is refused.
    pub fn add_peer(
        &self,
        node: &NodeId,
        key: &Ed25519PublicKey,
    ) -> Result<PeerOutcome, ShellError> {
        let mut peers = self.load_peers()?;
        match peers.get(node) {
            Some(existing) if existing == key => return Ok(PeerOutcome::AlreadyPresent),
            Some(_) => return Err(ShellError::PeerKeyConflict { node: node.clone() }),
            None => {}
        }
        peers.insert(node.clone(), *key);
        write_json(
            &self.dir.join(PEERS),
            &PeersFile {
                peers: peers
                    .iter()
                    .map(|(node, key)| (node.to_string(), key.to_string()))
                    .collect(),
            },
        )?;
        Ok(PeerOutcome::Added)
    }

    pub fn peers(&self) -> Result<Vec<(NodeId, Ed25519PublicKey)>, ShellError> {
        Ok(self.load_peers()?.into_iter().collect())
    }

    /// Sign and publish `envelope`: signature first, so a recipient never
    /// finds the message without its record. Recorded locally as sent.
    fn publish(&self, envelope: EnvelopeV0) -> Result<MessageId, ShellError> {
        self.relay
            .post_signature(&self.key.sign(self.node(), &envelope))?;
        self.relay.post_message(&envelope)?;
        let id = envelope.id().clone();
        self.messages()?.store(envelope).map_err(store_error)?;
        Ok(id)
    }

    /// Send a signed message to a trusted peer.
    pub fn send(
        &self,
        to: &NodeId,
        body: &str,
        correlation: Option<CorrelationId>,
        artifacts: &[ArtifactRef],
    ) -> Result<MessageId, ShellError> {
        if !self.load_peers()?.contains_key(to) {
            return Err(ShellError::UnknownPeer { node: to.clone() });
        }
        let mut envelope = EnvelopeV0::new(
            MessageId::generate(),
            self.node().clone(),
            to.clone(),
            MessageKind::Message,
            body,
        );
        if let Some(correlation) = correlation {
            envelope = envelope.with_correlation_id(correlation);
        }
        for artifact in artifacts {
            envelope = envelope.with_artifact_ref(artifact.clone());
        }
        self.publish(envelope)
    }

    /// Pull everything addressed to this node from the relay, verify it, and
    /// keep only what verifies. Already-kept messages are skipped.
    pub fn sync(&self) -> Result<SyncReport, ShellError> {
        let peers = self.load_peers()?;
        let mut messages = self.messages()?;
        let mut events = self.delivery_events()?;
        let mut report = SyncReport::default();

        for id in self.relay.list_for(self.node())? {
            if messages.load(&id).map_err(store_error)?.is_some() {
                continue;
            }
            let Some(envelope) = self.relay.get_message(&id)? else {
                continue;
            };
            let signatures = self.relay.signatures(&id)?;
            if let Err(rejection) = verify(
                &envelope,
                &signatures,
                peers.get(envelope.from()),
                self.node(),
            ) {
                report.rejected.push((id, rejection));
                continue;
            }

            if envelope.kind() == MessageKind::Acknowledgment {
                let Some(original) = self.acknowledged_message(&messages, &envelope)? else {
                    report
                        .rejected
                        .push((id, Rejection::UnmatchedAcknowledgment));
                    continue;
                };
                let by = envelope.from().clone();
                messages.store(envelope).map_err(store_error)?;
                record(
                    &mut events,
                    format!("acknowledged:{id}"),
                    &original,
                    &by,
                    DeliveryEventKind::RecipientAcknowledged,
                )?;
                report.acknowledged.push(Acknowledged {
                    ack: id,
                    message: original,
                    by,
                });
            } else {
                messages.store(envelope).map_err(store_error)?;
                for (prefix, kind) in [
                    ("received", DeliveryEventKind::RecipientReceived),
                    ("verified", DeliveryEventKind::RecipientVerified),
                ] {
                    record(
                        &mut events,
                        format!("{prefix}:{id}"),
                        &id,
                        self.node(),
                        kind,
                    )?;
                }
                report.received.push(id);
            }
        }
        Ok(report)
    }

    /// The message an ACK answers: one this node sent to the acknowledging
    /// peer, named by the ACK's correlation id.
    fn acknowledged_message(
        &self,
        messages: &SqliteMessageStore,
        ack: &EnvelopeV0,
    ) -> Result<Option<MessageId>, ShellError> {
        let Some(id) = ack
            .correlation_id()
            .and_then(|c| MessageId::parse(c.as_str()).ok())
        else {
            return Ok(None);
        };
        Ok(messages
            .load(&id)
            .map_err(store_error)?
            .filter(|original| {
                original.from() == self.node()
                    && original.to() == ack.from()
                    && original.kind() != MessageKind::Acknowledgment
            })
            .map(|_| id))
    }

    /// Verified messages received by this node, excluding acknowledgments.
    pub fn inbox(&self) -> Result<Vec<EnvelopeV0>, ShellError> {
        let messages = self.messages()?;
        let mut inbox = Vec::new();
        for id in messages
            .ids_for_recipient(self.node())
            .map_err(store_error)?
        {
            if let Some(envelope) = messages.load(&id).map_err(store_error)?
                && envelope.kind() != MessageKind::Acknowledgment
            {
                inbox.push(envelope);
            }
        }
        Ok(inbox)
    }

    /// Acknowledge receipt of a verified message from a peer. The ACK id is
    /// derived from the message, so repeating it is an exact replay.
    pub fn ack(&self, message_id: &MessageId) -> Result<MessageId, ShellError> {
        let not_ackable = || ShellError::NotAcknowledgeable {
            message_id: message_id.clone(),
        };
        let original = self
            .messages()?
            .load(message_id)
            .map_err(store_error)?
            .filter(|m| {
                m.to() == self.node()
                    && m.from() != self.node()
                    && m.kind() != MessageKind::Acknowledgment
            })
            .ok_or_else(not_ackable)?;
        let ack_id = MessageId::parse(format!("ack:{}:{message_id}", self.node()))
            .map_err(|_| not_ackable())?;
        let correlation = CorrelationId::parse(message_id.as_str()).map_err(|_| not_ackable())?;
        self.publish(
            EnvelopeV0::new(
                ack_id,
                self.node().clone(),
                original.from().clone(),
                MessageKind::Acknowledgment,
                "",
            )
            .with_correlation_id(correlation),
        )
    }

    /// Local delivery evidence for a message, in recording order.
    pub fn delivery(&self, message_id: &MessageId) -> Result<Vec<DeliveryEvent>, ShellError> {
        self.delivery_events()?
            .load_for_message(message_id)
            .map_err(store_error)
    }

    /// Store bytes locally and on the relay; both must agree on the ref.
    pub fn put_artifact(&self, bytes: &[u8]) -> Result<ArtifactRef, ShellError> {
        let local = match self.artifacts()?.put(bytes).map_err(store_error)? {
            ArtifactPutOutcome::Stored(artifact) | ArtifactPutOutcome::AlreadyPresent(artifact) => {
                artifact
            }
        };
        let remote = self.relay.put_artifact(bytes)?;
        if remote != local {
            return Err(ShellError::ArtifactMismatch { artifact: local });
        }
        Ok(local)
    }

    /// Fetch artifact bytes, locally if present, else from the relay. Relay
    /// bytes are kept only if they hash to the requested ref.
    pub fn get_artifact(&self, artifact: &ArtifactRef) -> Result<Vec<u8>, ShellError> {
        let mut store = self.artifacts()?;
        if let Some(bytes) = store.get(artifact).map_err(store_error)? {
            return Ok(bytes);
        }
        let bytes =
            self.relay
                .get_artifact(artifact)?
                .ok_or_else(|| ShellError::ArtifactNotFound {
                    artifact: artifact.clone(),
                })?;
        let stored = match store.put(&bytes).map_err(store_error)? {
            ArtifactPutOutcome::Stored(stored) | ArtifactPutOutcome::AlreadyPresent(stored) => {
                stored
            }
        };
        if &stored != artifact {
            return Err(ShellError::ArtifactMismatch {
                artifact: artifact.clone(),
            });
        }
        Ok(bytes)
    }
}

fn record(
    events: &mut SqliteDeliveryEventStore,
    event_id: String,
    message: &MessageId,
    reported_by: &NodeId,
    kind: DeliveryEventKind,
) -> Result<(), ShellError> {
    let id = DeliveryEventId::parse(event_id).map_err(store_error)?;
    events
        .append(DeliveryEvent::new(
            id,
            message.clone(),
            reported_by.clone(),
            kind,
        ))
        .map_err(store_error)?;
    Ok(())
}
