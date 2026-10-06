//! Deaddrop TUI V0: a small, keyboard-driven terminal view of one node — its
//! identity, trusted peers, and verified messages — over [`deaddrop_shell`].
//!
//! - [`input`]: terminal key events to app keys; which newline keys work.
//! - [`snapshot`]: network state, read only through the `Shell` API.
//! - [`app`]: UI state (focus, selection, vault section, first-seen order,
//!   unread, activity). Pure; no I/O.
//! - [`composer`]: how the compose draft wraps, grows, and scrolls.
//! - [`avatar`]: stable decorative avatars and spinner frames.
//! - [`theme`]: Night Garden, the app's own colors and background.
//! - [`ui`]: rendering. Draws state; never touches the network.
//!
//! Avatars and the spinner are UI decoration only. They are not presence or
//! any other protocol state.

pub mod app;
pub mod avatar;
pub mod clipboard;
pub mod composer;
pub mod input;
pub mod snapshot;
pub mod theme;
pub mod timing;
pub mod ui;
