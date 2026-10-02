//! Network shell commands: thin wrappers over `deaddrop_shell::Shell`.
//!
//! Every command takes `--home <dir>` (or `DEADDROP_HOME`) and prints one
//! JSON object on stdout. Slice 1 bodies are signed, not encrypted.

use std::collections::BTreeMap;
use std::path::PathBuf;

use deaddrop_protocol::{ArtifactRef, CorrelationId, Ed25519PublicKey, MessageId, NodeId};
use deaddrop_shell::{HttpRelay, PeerOutcome, Shell, ShellError, init};
use serde_json::{Value, json};

use crate::{EXIT_CONFLICT, EXIT_FAILURE, EXIT_INVALID, EXIT_NOT_FOUND, EXIT_USAGE, Failure};

pub const COMMANDS: &[&str] = &[
    "init", "identity", "peer", "peers", "send", "inbox", "ack", "status", "artifact",
];

/// Flags that take a value; `--artifact` may repeat.
const VALUE_FLAGS: &[&str] = &[
    "--home",
    "--node",
    "--relay",
    "--correlation",
    "--artifact",
    "--out",
];

struct Args {
    positional: Vec<String>,
    flags: BTreeMap<String, Vec<String>>,
}

impl Args {
    fn parse(args: &[String]) -> Result<Self, Failure> {
        let mut positional = Vec::new();
        let mut flags: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg.starts_with("--") {
                if !VALUE_FLAGS.contains(&arg.as_str()) {
                    return Err(usage(format!("unexpected argument {arg:?}")));
                }
                let value = iter
                    .next()
                    .ok_or_else(|| usage(format!("missing value for {arg}")))?;
                flags.entry(arg.clone()).or_default().push(value.clone());
            } else {
                positional.push(arg.clone());
            }
        }
        for (flag, values) in &flags {
            if flag != "--artifact" && values.len() > 1 {
                return Err(usage(format!("duplicate {flag}")));
            }
        }
        Ok(Self { positional, flags })
    }

    fn one(&self, flag: &str) -> Option<&str> {
        self.flags.get(flag).map(|values| values[0].as_str())
    }

    fn required(&self, flag: &str) -> Result<&str, Failure> {
        self.one(flag)
            .ok_or_else(|| usage(format!("missing required {flag}")))
    }

    fn positionals<const N: usize>(&self) -> Result<[&str; N], Failure> {
        let values: Vec<&str> = self.positional.iter().map(String::as_str).collect();
        values
            .try_into()
            .map_err(|_| usage(format!("expected {N} positional argument(s)")))
    }

    fn home(&self) -> Result<PathBuf, Failure> {
        self.one("--home")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("DEADDROP_HOME").map(PathBuf::from))
            .ok_or_else(|| usage("missing --home (or DEADDROP_HOME)"))
    }
}

fn usage(message: impl Into<String>) -> Failure {
    Failure::new(EXIT_USAGE, message)
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(EXIT_INVALID, message)
}

fn shell_failure(error: ShellError) -> Failure {
    let code = match &error {
        ShellError::AlreadyInitialized | ShellError::PeerKeyConflict { .. } => EXIT_CONFLICT,
        ShellError::NotInitialized
        | ShellError::InvalidRelay { .. }
        | ShellError::NonLoopbackRelay { .. }
        | ShellError::UnknownPeer { .. }
        | ShellError::NotAcknowledgeable { .. }
        | ShellError::ArtifactMismatch { .. } => EXIT_INVALID,
        ShellError::ArtifactNotFound { .. } => EXIT_NOT_FOUND,
        _ => EXIT_FAILURE,
    };
    Failure::new(code, error.to_string())
}

fn node_id(value: &str) -> Result<NodeId, Failure> {
    NodeId::parse(value).map_err(|error| invalid(format!("invalid node id {value:?}: {error}")))
}

fn message_id(value: &str) -> Result<MessageId, Failure> {
    MessageId::parse(value)
        .map_err(|error| invalid(format!("invalid message id {value:?}: {error}")))
}

fn artifact_ref(value: &str) -> Result<ArtifactRef, Failure> {
    value
        .parse()
        .map_err(|error| invalid(format!("invalid artifact ref {value:?}: {error}")))
}

fn open(args: &Args) -> Result<Shell<HttpRelay>, Failure> {
    Shell::open(&args.home()?).map_err(shell_failure)
}

pub fn run(command: &str, rest: &[String]) -> Result<Value, Failure> {
    let args = Args::parse(rest)?;
    match command {
        "init" => {
            args.positionals::<0>()?;
            let card = init(
                &args.home()?,
                node_id(args.required("--node")?)?,
                args.required("--relay")?,
            )
            .map_err(shell_failure)?;
            Ok(
                json!({ "node_id": card.node.as_str(), "key": card.key.to_string(), "relay": card.relay }),
            )
        }
        "identity" => {
            args.positionals::<0>()?;
            let card = open(&args)?.identity();
            Ok(
                json!({ "node_id": card.node.as_str(), "key": card.key.to_string(), "relay": card.relay }),
            )
        }
        "peer" => {
            let [action, node, key] = args.positionals::<3>()?;
            if action != "add" {
                return Err(usage(format!("unknown peer action {action:?}")));
            }
            let node = node_id(node)?;
            let key: Ed25519PublicKey = key
                .parse()
                .map_err(|error| invalid(format!("invalid key {key:?}: {error}")))?;
            let outcome = open(&args)?.add_peer(&node, &key).map_err(shell_failure)?;
            let outcome = match outcome {
                PeerOutcome::Added => "added",
                PeerOutcome::AlreadyPresent => "already_present",
            };
            Ok(json!({ "node_id": node.as_str(), "key": key.to_string(), "outcome": outcome }))
        }
        "peers" => {
            args.positionals::<0>()?;
            let peers: Vec<Value> = open(&args)?
                .peers()
                .map_err(shell_failure)?
                .into_iter()
                .map(|(node, key)| json!({ "node_id": node.as_str(), "key": key.to_string() }))
                .collect();
            Ok(json!({ "peers": peers }))
        }
        "send" => {
            let [peer, body] = args.positionals::<2>()?;
            let correlation = args
                .one("--correlation")
                .map(|value| {
                    CorrelationId::parse(value)
                        .map_err(|error| invalid(format!("invalid correlation {value:?}: {error}")))
                })
                .transpose()?;
            let artifacts = args
                .flags
                .get("--artifact")
                .into_iter()
                .flatten()
                .map(|value| artifact_ref(value))
                .collect::<Result<Vec<_>, _>>()?;
            let id = open(&args)?
                .send(&node_id(peer)?, body, correlation, &artifacts)
                .map_err(shell_failure)?;
            Ok(json!({ "message_id": id.as_str() }))
        }
        "inbox" => {
            args.positionals::<0>()?;
            let shell = open(&args)?;
            let report = shell.sync().map_err(shell_failure)?;
            let messages: Vec<Value> = shell
                .inbox()
                .map_err(shell_failure)?
                .iter()
                .map(|m| {
                    json!({
                        "id": m.id().as_str(),
                        "from": m.from().as_str(),
                        "kind": m.kind().as_str(),
                        "correlation_id": m.correlation_id().map(|c| c.as_str()),
                        "body": m.body(),
                        "artifact_refs": m.artifact_refs().iter().map(ToString::to_string).collect::<Vec<_>>(),
                    })
                })
                .collect();
            Ok(json!({
                "received": report.received.iter().map(MessageId::as_str).collect::<Vec<_>>(),
                "acknowledged": report.acknowledged.iter().map(|a| json!({
                    "ack_id": a.ack.as_str(),
                    "message_id": a.message.as_str(),
                    "by": a.by.as_str(),
                })).collect::<Vec<_>>(),
                "rejected": report.rejected.iter().map(|(id, reason)| json!({
                    "message_id": id.as_str(),
                    "reason": reason.as_str(),
                })).collect::<Vec<_>>(),
                "messages": messages,
            }))
        }
        "ack" => {
            let [id] = args.positionals::<1>()?;
            let id = message_id(id)?;
            let ack = open(&args)?.ack(&id).map_err(shell_failure)?;
            Ok(json!({ "ack_id": ack.as_str(), "message_id": id.as_str() }))
        }
        "status" => {
            let [id] = args.positionals::<1>()?;
            let id = message_id(id)?;
            let events = open(&args)?.delivery(&id).map_err(shell_failure)?;
            let acknowledged = events
                .iter()
                .any(|e| e.kind() == deaddrop_protocol::DeliveryEventKind::RecipientAcknowledged);
            Ok(json!({
                "message_id": id.as_str(),
                "events": events.iter().map(|e| json!({
                    "kind": e.kind().as_str(),
                    "reported_by": e.reported_by().as_str(),
                })).collect::<Vec<_>>(),
                "acknowledged": acknowledged,
            }))
        }
        "artifact" => match args.positional.first().map(String::as_str) {
            Some("put") => {
                let [_, file] = args.positionals::<2>()?;
                let bytes = std::fs::read(file).map_err(|error| {
                    Failure::new(EXIT_FAILURE, format!("read {file:?}: {error}"))
                })?;
                let artifact = open(&args)?.put_artifact(&bytes).map_err(shell_failure)?;
                Ok(json!({ "artifact_ref": artifact.to_string() }))
            }
            Some("get") => {
                let [_, value] = args.positionals::<2>()?;
                let artifact = artifact_ref(value)?;
                let out = args.required("--out")?;
                let bytes = open(&args)?
                    .get_artifact(&artifact)
                    .map_err(shell_failure)?;
                std::fs::write(out, &bytes).map_err(|error| {
                    Failure::new(EXIT_FAILURE, format!("write {out:?}: {error}"))
                })?;
                Ok(
                    json!({ "artifact_ref": artifact.to_string(), "out": out, "bytes": bytes.len() }),
                )
            }
            _ => Err(usage(
                "expected: artifact put <file> | artifact get <ref> --out <path>",
            )),
        },
        other => Err(usage(format!("unknown command {other:?}"))),
    }
}
