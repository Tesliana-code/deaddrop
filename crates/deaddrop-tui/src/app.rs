//! UI state, kept apart from network state. Pure: no terminal, no I/O.
//!
//! Network state arrives only as a [`Snapshot`] from a refresh; everything
//! else here (focus, selection, view, activity) belongs to the UI.

use crate::snapshot::{Peer, Received, Sent, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Esc,
    Tab,
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Peers,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Detail,
}

/// What the UI is waiting on. A UI label only, never protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Busy(&'static str),
}

/// One line in the messages pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    /// A verified message received by this node.
    Received(&'a Received),
    /// A message this node sent, with its recorded ACK status.
    Sent(&'a Sent),
}

impl Row<'_> {
    /// The node on the other end.
    pub fn peer(&self) -> &str {
        match self {
            Self::Received(m) => &m.from,
            Self::Sent(m) => &m.to,
        }
    }

    fn key(&self) -> &str {
        match self {
            Self::Received(m) => &m.id,
            Self::Sent(m) => &m.id,
        }
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub snapshot: Option<Snapshot>,
    pub focus: Focus,
    pub view: View,
    pub peer: usize,
    pub message: usize,
    pub activity: Activity,
    pub status: String,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            focus: Focus::Peers,
            view: View::List,
            peer: 0,
            message: 0,
            activity: Activity::Idle,
            status: "starting".into(),
        }
    }

    pub fn peers(&self) -> &[Peer] {
        self.snapshot.as_ref().map_or(&[], |s| &s.peers)
    }

    /// Received messages, then sent ones.
    pub fn messages(&self) -> Vec<Row<'_>> {
        let Some(s) = &self.snapshot else {
            return Vec::new();
        };
        s.inbox
            .iter()
            .map(Row::Received)
            .chain(s.sent.iter().map(Row::Sent))
            .collect()
    }

    pub fn selected_peer(&self) -> Option<&Peer> {
        self.peers().get(self.peer)
    }

    pub fn selected_message(&self) -> Option<Row<'_>> {
        self.messages().get(self.message).copied()
    }

    /// Messages exchanged with one peer.
    pub fn messages_with(&self, peer: &str) -> Vec<Row<'_>> {
        self.messages()
            .into_iter()
            .filter(|row| row.peer() == peer)
            .collect()
    }

    pub fn busy(&self) -> bool {
        matches!(self.activity, Activity::Busy(_))
    }

    pub fn key(&mut self, key: Key) -> Action {
        match (self.view, key) {
            (_, Key::Char('q')) => return Action::Quit,
            (View::Detail, Key::Esc) => self.view = View::List,
            (View::List, Key::Esc) => return Action::Quit,
            (_, Key::Char('r')) => return Action::Refresh,
            (View::List, Key::Tab | Key::Char('h' | 'l')) => {
                self.focus = match self.focus {
                    Focus::Peers => Focus::Messages,
                    Focus::Messages => Focus::Peers,
                }
            }
            (View::List, Key::Up | Key::Char('k')) => self.step(-1),
            (View::List, Key::Down | Key::Char('j')) => self.step(1),
            (View::List, Key::Enter) if self.focused_len() > 0 => self.view = View::Detail,
            _ => {}
        }
        Action::None
    }

    fn focused_len(&self) -> usize {
        match self.focus {
            Focus::Peers => self.peers().len(),
            Focus::Messages => self.messages().len(),
        }
    }

    fn step(&mut self, delta: isize) {
        let len = self.focused_len();
        let index = match self.focus {
            Focus::Peers => &mut self.peer,
            Focus::Messages => &mut self.message,
        };
        *index = match len {
            0 => 0,
            _ => index.saturating_add_signed(delta).min(len - 1),
        };
    }

    /// Start a refresh unless one is already running.
    pub fn begin_refresh(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        self.activity = Activity::Busy("syncing");
        true
    }

    pub fn progress(&mut self, label: &'static str) {
        if self.busy() {
            self.activity = Activity::Busy(label);
        }
    }

    /// Apply a finished refresh. A failure keeps the previous snapshot.
    pub fn finish(&mut self, result: Result<Snapshot, String>) {
        self.activity = Activity::Idle;
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.status = format!("refresh failed: {error}");
                return;
            }
        };
        let selected_peer = self.selected_peer().map(|p| p.id.clone());
        let selected_message = self.selected_message().map(|m| m.key().to_owned());

        self.status = match &snapshot.sync {
            Ok(sync) => format!(
                "synced: {} new, {} ack, {} rejected",
                sync.received, sync.acknowledged, sync.rejected
            ),
            Err(error) => format!("relay unreachable, showing local state: {error}"),
        };
        self.snapshot = Some(snapshot);

        self.peer = reselect(
            self.peers().iter().map(|p| p.id.as_str()),
            selected_peer,
            self.peer,
        );
        let message_keys: Vec<String> =
            self.messages().iter().map(|m| m.key().to_owned()).collect();
        self.message = reselect(
            message_keys.iter().map(String::as_str),
            selected_message,
            self.message,
        );
        if self.focused_len() == 0 {
            self.view = View::List;
        }
    }
}

/// Keep the same item selected if it is still there, else clamp.
fn reselect<'a>(
    keys: impl ExactSizeIterator<Item = &'a str>,
    selected: Option<String>,
    index: usize,
) -> usize {
    let len = keys.len();
    let mut keys = keys;
    selected
        .and_then(|s| keys.position(|k| k == s))
        .unwrap_or_else(|| index.min(len.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{Node, SyncSummary};

    fn peer(id: &str) -> Peer {
        Peer {
            id: id.into(),
            key: format!("ed25519:{id}"),
        }
    }

    fn received(id: &str, from: &str) -> Received {
        Received {
            id: id.into(),
            from: from.into(),
            kind: "message".into(),
            body: format!("hello from {from}"),
            correlation: None,
            artifacts: vec![],
            delivery: vec!["recipient_received".into(), "recipient_verified".into()],
        }
    }

    fn sent(id: &str, to: &str, acked: bool) -> Sent {
        Sent {
            id: id.into(),
            to: to.into(),
            kind: "message".into(),
            body: format!("hello to {to}"),
            correlation: None,
            artifacts: vec![],
            delivery: if acked {
                vec!["recipient_acknowledged".into()]
            } else {
                vec![]
            },
            acked_by: if acked { vec![to.into()] } else { vec![] },
        }
    }

    fn snapshot(peers: &[&str], inbox: &[(&str, &str)], sent: &[Sent]) -> Snapshot {
        Snapshot {
            node: Node {
                id: "me:shell:deaddrop".into(),
                key: "ed25519:me".into(),
                relay: "http://127.0.0.1:8787".into(),
            },
            peers: peers.iter().map(|p| peer(p)).collect(),
            inbox: inbox.iter().map(|(id, from)| received(id, from)).collect(),
            sent: sent.to_vec(),
            sync: Ok(SyncSummary {
                received: inbox.len(),
                acknowledged: 0,
                rejected: 0,
            }),
        }
    }

    fn loaded(s: Snapshot) -> App {
        let mut app = App::new();
        assert!(app.begin_refresh());
        app.finish(Ok(s));
        app
    }

    #[test]
    fn empty_state_is_navigable() {
        let mut app = loaded(snapshot(&[], &[], &[]));
        for key in [
            Key::Up,
            Key::Down,
            Key::Enter,
            Key::Tab,
            Key::Down,
            Key::Enter,
        ] {
            assert_eq!(app.key(key), Action::None);
        }
        assert_eq!(app.view, View::List);
        assert_eq!((app.peer, app.message), (0, 0));
        assert!(app.selected_peer().is_none());
        assert!(app.selected_message().is_none());
    }

    #[test]
    fn no_snapshot_yet_is_empty() {
        let mut app = App::new();
        assert!(app.peers().is_empty() && app.messages().is_empty());
        assert_eq!(app.key(Key::Down), Action::None);
    }

    #[test]
    fn selection_moves_and_clamps_per_pane() {
        let mut app = loaded(snapshot(&["a", "b", "c"], &[("m1", "a"), ("m2", "b")], &[]));
        app.key(Key::Up);
        assert_eq!(app.peer, 0);
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Down);
        assert_eq!(app.selected_peer().unwrap().id, "c");

        app.key(Key::Tab);
        assert_eq!(app.focus, Focus::Messages);
        app.key(Key::Down);
        app.key(Key::Down);
        assert_eq!(app.message, 1);
        assert_eq!(app.peer, 2, "peer selection is independent");
    }

    #[test]
    fn enter_opens_detail_and_esc_returns_then_quits() {
        let mut app = loaded(snapshot(&["a"], &[], &[]));
        assert_eq!(app.key(Key::Enter), Action::None);
        assert_eq!(app.view, View::Detail);
        assert_eq!(app.key(Key::Down), Action::None, "no movement in detail");
        assert_eq!(app.key(Key::Esc), Action::None);
        assert_eq!(app.view, View::List);
        assert_eq!(app.key(Key::Esc), Action::Quit);
    }

    #[test]
    fn q_quits_from_any_view() {
        let mut app = loaded(snapshot(&["a"], &[], &[]));
        app.key(Key::Enter);
        assert_eq!(app.key(Key::Char('q')), Action::Quit);
    }

    #[test]
    fn refresh_runs_one_at_a_time() {
        let mut app = App::new();
        assert_eq!(app.key(Key::Char('r')), Action::Refresh);
        assert!(app.begin_refresh());
        assert!(!app.begin_refresh());
        app.progress("verifying");
        assert_eq!(app.activity, Activity::Busy("verifying"));
        app.finish(Ok(snapshot(&[], &[], &[])));
        assert_eq!(app.activity, Activity::Idle);
        app.progress("verifying");
        assert_eq!(app.activity, Activity::Idle, "no progress while idle");
    }

    #[test]
    fn failed_refresh_keeps_previous_state() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        app.begin_refresh();
        app.finish(Err("boom".into()));
        assert_eq!(app.activity, Activity::Idle);
        assert!(app.status.contains("boom"));
        assert_eq!(app.peers().len(), 1);
        assert_eq!(app.messages().len(), 1);
    }

    #[test]
    fn unreachable_relay_still_shows_local_state() {
        let mut s = snapshot(&["a"], &[("m1", "a")], &[]);
        s.sync = Err("connection refused".into());
        let app = loaded(s);
        assert!(app.status.contains("relay unreachable"));
        assert_eq!(app.messages().len(), 1);
    }

    #[test]
    fn incoming_then_outgoing_rows_stay_distinct() {
        let app = loaded(snapshot(&["a"], &[("m1", "a")], &[sent("s1", "a", false)]));
        let rows = app.messages();
        assert!(matches!(rows[0], Row::Received(m) if m.from == "a"));
        assert!(matches!(rows[1], Row::Sent(m) if m.to == "a"));
    }

    #[test]
    fn ack_status_follows_the_snapshot_not_session_memory() {
        // A fresh App (as after restart) shows ACK status from the snapshot
        // alone, with no sync having reported it.
        let mut s = snapshot(&["a"], &[], &[sent("s1", "a", true)]);
        s.sync = Err("relay down".into());
        let app = loaded(s);
        assert!(matches!(app.messages()[0], Row::Sent(m) if m.acked_by == ["a"]));
    }

    #[test]
    fn repeated_refreshes_do_not_duplicate_rows() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[sent("s1", "a", true)]));
        for _ in 0..3 {
            app.begin_refresh();
            app.finish(Ok(snapshot(
                &["a"],
                &[("m1", "a")],
                &[sent("s1", "a", true)],
            )));
        }
        assert_eq!(app.messages().len(), 2);
    }

    #[test]
    fn refresh_keeps_selected_item_by_id() {
        let mut app = loaded(snapshot(&["b", "c"], &[("m2", "b")], &[]));
        app.key(Key::Down);
        assert_eq!(app.selected_peer().unwrap().id, "c");
        app.begin_refresh();
        app.finish(Ok(snapshot(
            &["a", "b", "c"],
            &[("m1", "a"), ("m2", "b")],
            &[],
        )));
        assert_eq!(app.selected_peer().unwrap().id, "c");
    }

    #[test]
    fn refresh_clamps_when_selection_disappears() {
        let mut app = loaded(snapshot(&["a", "b", "c"], &[], &[]));
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Enter);
        app.begin_refresh();
        app.finish(Ok(snapshot(&["a"], &[], &[])));
        assert_eq!(app.selected_peer().unwrap().id, "a");
        app.begin_refresh();
        app.finish(Ok(snapshot(&[], &[], &[])));
        assert_eq!(app.peer, 0);
        assert_eq!(app.view, View::List, "detail of nothing closes");
    }

    #[test]
    fn conversation_filters_by_peer() {
        let app = loaded(snapshot(
            &["a", "b"],
            &[("m1", "a"), ("m2", "b"), ("m3", "a")],
            &[sent("s1", "a", true), sent("s2", "b", false)],
        ));
        let rows = app.messages_with("a");
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.peer() == "a"));
    }
}
