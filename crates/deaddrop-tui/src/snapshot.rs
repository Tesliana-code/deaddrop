//! Network state, read through the existing [`Shell`] API only.
//!
//! A [`Snapshot`] is plain display data. The TUI never verifies, signs, or
//! talks to the relay itself; `Shell::sync` does all of that.

use deaddrop_protocol::{DeliveryEventKind, EnvelopeV0, encode_envelope_v0};
use deaddrop_shell::{Relay, Shell, ShellError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    pub key: String,
    pub relay: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    pub key: String,
}

/// A verified message received by this node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub id: String,
    pub from: String,
    pub kind: String,
    pub body: String,
    pub correlation: Option<String>,
    pub artifacts: Vec<String>,
    /// Local delivery evidence kinds, in recording order.
    pub delivery: Vec<String>,
    /// The canonical V0 wire encoding, for inspection only.
    pub raw: String,
}

/// A message this node sent. ACK status comes from durable delivery
/// evidence, so it is the same after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub id: String,
    pub to: String,
    pub kind: String,
    pub body: String,
    pub correlation: Option<String>,
    pub artifacts: Vec<String>,
    /// Local delivery evidence kinds, in recording order.
    pub delivery: Vec<String>,
    /// Peers whose verified ACK is recorded, each once.
    pub acked_by: Vec<String>,
    /// The canonical V0 wire encoding, for inspection only.
    pub raw: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncSummary {
    pub received: usize,
    pub acknowledged: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub node: Node,
    pub peers: Vec<Peer>,
    pub inbox: Vec<Received>,
    pub sent: Vec<Sent>,
    /// `Err` holds why the relay pass failed; local state is still shown.
    pub sync: Result<SyncSummary, String>,
}

/// Sync with the relay, then read local state. A failed sync does not hide
/// what the node already holds.
pub fn load<R: Relay>(shell: &Shell<R>) -> Result<Snapshot, ShellError> {
    let sync = shell
        .sync()
        .map(|report| SyncSummary {
            received: report.received.len(),
            acknowledged: report.acknowledged.len(),
            rejected: report.rejected.len(),
        })
        .map_err(|e| e.to_string());
    let card = shell.identity();
    let peers = shell
        .peers()?
        .into_iter()
        .map(|(id, key)| Peer {
            id: id.to_string(),
            key: key.to_string(),
        })
        .collect();
    let inbox = shell
        .inbox()?
        .iter()
        .map(|envelope| received(shell, envelope))
        .collect::<Result<_, _>>()?;
    let sent = shell
        .sent()?
        .iter()
        .map(|envelope| sent(shell, envelope))
        .collect::<Result<_, _>>()?;
    Ok(Snapshot {
        node: Node {
            id: card.node.to_string(),
            key: card.key.to_string(),
            relay: card.relay,
        },
        peers,
        inbox,
        sent,
        sync,
    })
}

fn received<R: Relay>(shell: &Shell<R>, envelope: &EnvelopeV0) -> Result<Received, ShellError> {
    Ok(Received {
        id: envelope.id().to_string(),
        from: envelope.from().to_string(),
        kind: envelope.kind().as_str().to_owned(),
        body: envelope.body().to_owned(),
        correlation: envelope.correlation_id().map(|c| c.to_string()),
        artifacts: envelope
            .artifact_refs()
            .iter()
            .map(|a| a.to_string())
            .collect(),
        delivery: shell
            .delivery(envelope.id())?
            .iter()
            .map(|e| e.kind().as_str().to_owned())
            .collect(),
        raw: raw(envelope),
    })
}

fn sent<R: Relay>(shell: &Shell<R>, envelope: &EnvelopeV0) -> Result<Sent, ShellError> {
    let events = shell.delivery(envelope.id())?;
    let mut acked_by: Vec<String> = Vec::new();
    for event in &events {
        let by = event.reported_by().to_string();
        if event.kind() == DeliveryEventKind::RecipientAcknowledged && !acked_by.contains(&by) {
            acked_by.push(by);
        }
    }
    Ok(Sent {
        id: envelope.id().to_string(),
        to: envelope.to().to_string(),
        kind: envelope.kind().as_str().to_owned(),
        body: envelope.body().to_owned(),
        correlation: envelope.correlation_id().map(|c| c.to_string()),
        artifacts: envelope
            .artifact_refs()
            .iter()
            .map(|a| a.to_string())
            .collect(),
        delivery: events
            .iter()
            .map(|e| e.kind().as_str().to_owned())
            .collect(),
        acked_by,
        raw: raw(envelope),
    })
}

fn raw(envelope: &EnvelopeV0) -> String {
    encode_envelope_v0(envelope)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_else(|e| format!("<not encodable: {e}>"))
}
