//! Deaddrop network shell V0: a local node's identity, explicit peers, signed
//! asynchronous messages, protocol acknowledgments, and immutable artifact
//! references, over a replaceable mailbox relay.
//!
//! - [`key`]: the node's Ed25519 key (ADR-002) and detached signing.
//! - [`verify`]: recipient verification rules. Pure; no I/O.
//! - [`relay`]: the [`Relay`] transport seam, its HTTP client for the local
//!   `deaddrop-node` mailbox, and an in-process [`MemoryRelay`].
//! - [`shell`]: [`Shell`], the node-local operations behind the CLI.
//!
//! Boundaries:
//! - The relay stores and returns opaque records. It never verifies, and it
//!   is never an identity authority; every node verifies for itself.
//! - An acknowledgment is protocol receipt only, never task success.
//! - No coordination, orchestration, or agent semantics live here.
//!
//! **Slice 1 sends plaintext bodies.** Messages are signed, not encrypted.
//! [`init`] therefore refuses any relay that is not loopback: encryption is
//! required before a non-loopback relay is treated as production-capable.

pub mod key;
pub mod relay;
pub mod shell;
pub mod verify;

pub use key::LocalKey;
pub use relay::{HttpRelay, MemoryRelay, Relay, RelayError};
pub use shell::{Acknowledged, IdentityCard, PeerOutcome, Shell, ShellError, SyncReport, init};
pub use verify::{Rejection, verify};
