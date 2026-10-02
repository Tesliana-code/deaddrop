//! Deaddrop TUI V0: a small, keyboard-driven terminal view of one node — its
//! identity, trusted peers, and verified messages — over [`deaddrop_shell`].
//!
//! - [`snapshot`]: network state, read only through the `Shell` API.
//! - [`app`]: UI state (focus, selection, vault section, first-seen order,
//!   unread, activity). Pure; no I/O.
//! - [`avatar`]: stable decorative avatars and spinner frames.
//! - [`ui`]: rendering. Draws state; never touches the network.
//!
//! Avatars and the spinner are UI decoration only. They are not presence or
//! any other protocol state.

pub mod app;
pub mod avatar;
pub mod snapshot;
pub mod ui;
