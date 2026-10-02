//! UI state, kept apart from network state. Pure: no terminal, no I/O.
//!
//! Network state arrives only as a [`Snapshot`] from a refresh. Everything
//! else here belongs to the UI: focus, selection, which vault section is
//! open, the order messages were first seen in, and which arrivals have not
//! been looked at yet. None of it is protocol state.

use std::collections::{BTreeSet, HashMap};

use crate::snapshot::{Peer, Received, Sent, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Tab,
    BackTab,
    Char(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
}

/// Left to right: who, what was said, what it is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Contacts,
    Conversation,
    Vault,
}

/// What the UI is waiting on. A UI label only, never protocol state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Busy(&'static str),
}

/// Inspectable facets of one message, in vault order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Identity,
    Trust,
    Signatures,
    Artifacts,
    Delivery,
    Correlation,
    Raw,
}

impl Section {
    pub const ALL: [Section; 7] = [
        Self::Identity,
        Self::Trust,
        Self::Signatures,
        Self::Artifacts,
        Self::Delivery,
        Self::Correlation,
        Self::Raw,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Trust => "trust",
            Self::Signatures => "signatures",
            Self::Artifacts => "artifacts",
            Self::Delivery => "delivery",
            Self::Correlation => "correlation",
            Self::Raw => "raw envelope",
        }
    }
}

/// One message in a conversation, either direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    /// A verified message received by this node.
    Received(&'a Received),
    /// A message this node sent, with its recorded ACK status.
    Sent(&'a Sent),
}

impl<'a> Row<'a> {
    pub fn id(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.id,
            Self::Sent(m) => &m.id,
        }
    }

    /// The node on the other end.
    pub fn peer(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.from,
            Self::Sent(m) => &m.to,
        }
    }

    pub fn outgoing(&self) -> bool {
        matches!(self, Self::Sent(_))
    }

    pub fn body(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.body,
            Self::Sent(m) => &m.body,
        }
    }

    pub fn kind(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.kind,
            Self::Sent(m) => &m.kind,
        }
    }

    pub fn correlation(&self) -> Option<&'a str> {
        match self {
            Self::Received(m) => m.correlation.as_deref(),
            Self::Sent(m) => m.correlation.as_deref(),
        }
    }

    pub fn artifacts(&self) -> &'a [String] {
        match self {
            Self::Received(m) => &m.artifacts,
            Self::Sent(m) => &m.artifacts,
        }
    }

    pub fn delivery(&self) -> &'a [String] {
        match self {
            Self::Received(m) => &m.delivery,
            Self::Sent(m) => &m.delivery,
        }
    }

    pub fn raw(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.raw,
            Self::Sent(m) => &m.raw,
        }
    }

    /// Sent and no ACK recorded yet.
    pub fn awaiting_ack(&self) -> bool {
        matches!(self, Self::Sent(m) if m.acked_by.is_empty())
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub snapshot: Option<Snapshot>,
    pub focus: Focus,
    pub contact: usize,
    /// Index into the selected contact's thread.
    pub message: usize,
    pub section: usize,
    pub activity: Activity,
    pub status: String,
    /// Whether the last refresh reached the relay; `None` before the first.
    pub relay_ok: Option<bool>,
    /// Message ids in the order this UI first saw them. V0 envelopes carry
    /// no time, so this is the only order there is.
    order: Vec<String>,
    /// `order[..earlier]` was already stored when the UI started.
    earlier: usize,
    /// Received this session and not looked at yet.
    unread: BTreeSet<String>,
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
            focus: Focus::Contacts,
            contact: 0,
            message: 0,
            section: 0,
            activity: Activity::Idle,
            status: "waking up".into(),
            relay_ok: None,
            order: Vec::new(),
            earlier: 0,
            unread: BTreeSet::new(),
        }
    }

    pub fn peers(&self) -> &[Peer] {
        self.snapshot.as_ref().map_or(&[], |s| &s.peers)
    }

    fn rows(&self) -> Vec<Row<'_>> {
        let Some(s) = &self.snapshot else {
            return Vec::new();
        };
        s.inbox
            .iter()
            .map(Row::Received)
            .chain(s.sent.iter().map(Row::Sent))
            .collect()
    }

    /// Messages with one peer, both directions, in first-seen order.
    pub fn thread(&self, peer: &str) -> Vec<Row<'_>> {
        let position: HashMap<&str, usize> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut rows: Vec<_> = self
            .rows()
            .into_iter()
            .filter(|r| r.peer() == peer)
            .collect();
        rows.sort_by_key(|r| position.get(r.id()).copied().unwrap_or(usize::MAX));
        rows
    }

    /// True if the message was already stored when this UI started, so its
    /// place in the thread says nothing about when it was sent.
    pub fn is_earlier(&self, id: &str) -> bool {
        self.order[..self.earlier].iter().any(|o| o == id)
    }

    pub fn is_unread(&self, id: &str) -> bool {
        self.unread.contains(id)
    }

    pub fn selected_peer(&self) -> Option<&Peer> {
        self.peers().get(self.contact)
    }

    pub fn current_thread(&self) -> Vec<Row<'_>> {
        self.selected_peer()
            .map_or_else(Vec::new, |p| self.thread(&p.id))
    }

    pub fn selected_message(&self) -> Option<Row<'_>> {
        self.current_thread().get(self.message).copied()
    }

    pub fn section(&self) -> Section {
        Section::ALL[self.section]
    }

    pub fn unread_with(&self, peer: &str) -> usize {
        self.thread(peer)
            .iter()
            .filter(|r| self.is_unread(r.id()))
            .count()
    }

    pub fn awaiting_with(&self, peer: &str) -> usize {
        self.thread(peer)
            .iter()
            .filter(|r| r.awaiting_ack())
            .count()
    }

    pub fn unread_total(&self) -> usize {
        self.unread.len()
    }

    pub fn awaiting_total(&self) -> usize {
        self.rows().iter().filter(|r| r.awaiting_ack()).count()
    }

    pub fn busy(&self) -> bool {
        matches!(self.activity, Activity::Busy(_))
    }

    pub fn key(&mut self, key: Key) -> Action {
        use Focus::*;
        match (self.focus, key) {
            (_, Key::Char('q')) => return Action::Quit,
            (_, Key::Char('r')) => return Action::Refresh,
            (Contacts, Key::Esc) => return Action::Quit,

            (Contacts, Key::Up | Key::Char('k')) => self.pick_contact(-1),
            (Contacts, Key::Down | Key::Char('j')) => self.pick_contact(1),
            (Contacts, Key::Enter | Key::Right | Key::Tab | Key::Char('l')) => {
                if self.selected_peer().is_some() {
                    self.focus_on(Conversation);
                }
            }

            (Conversation, Key::Up | Key::Char('k')) => self.pick_message(-1),
            (Conversation, Key::Down | Key::Char('j')) => self.pick_message(1),
            (Conversation, Key::Enter | Key::Right | Key::Tab | Key::Char('l')) => {
                if self.selected_message().is_some() {
                    self.focus_on(Vault);
                }
            }
            (Conversation, Key::Esc | Key::Left | Key::BackTab | Key::Char('h')) => {
                self.focus_on(Contacts)
            }

            (Vault, Key::Up | Key::Char('k')) => {
                self.section = self.section.saturating_sub(1);
            }
            (Vault, Key::Down | Key::Char('j')) => {
                self.section = (self.section + 1).min(Section::ALL.len() - 1);
            }
            (Vault, Key::Esc | Key::Left | Key::BackTab | Key::Char('h')) => {
                self.focus_on(Conversation)
            }
            (Vault, Key::Tab) => self.focus_on(Contacts),
            _ => {}
        }
        Action::None
    }

    fn focus_on(&mut self, focus: Focus) {
        self.focus = focus;
        self.mark_read();
    }

    /// Looking at a conversation reads it.
    fn mark_read(&mut self) {
        if self.focus == Focus::Contacts {
            return;
        }
        let ids: Vec<String> = self
            .current_thread()
            .iter()
            .map(|r| r.id().to_owned())
            .collect();
        for id in ids {
            self.unread.remove(&id);
        }
    }

    fn pick_contact(&mut self, delta: isize) {
        let len = self.peers().len();
        if len == 0 {
            return;
        }
        let next = self.contact.saturating_add_signed(delta).min(len - 1);
        if next != self.contact {
            self.contact = next;
            self.message = self.current_thread().len().saturating_sub(1);
        }
    }

    fn pick_message(&mut self, delta: isize) {
        let len = self.current_thread().len();
        self.message = match len {
            0 => 0,
            _ => self.message.saturating_add_signed(delta).min(len - 1),
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
        let first = self.snapshot.is_none();
        let selected_peer = self.selected_peer().map(|p| p.id.clone());
        let selected_message = self.selected_message().map(|m| m.id().to_owned());
        let was_at_end = self.message + 1 >= self.current_thread().len();

        self.relay_ok = Some(snapshot.sync.is_ok());
        self.status = match &snapshot.sync {
            Ok(sync) => format!(
                "synced · {} new · {} ack · {} rejected",
                sync.received, sync.acknowledged, sync.rejected
            ),
            Err(error) => format!("relay unreachable · showing local state · {error}"),
        };

        let incoming: Vec<(String, bool)> = snapshot
            .inbox
            .iter()
            .map(|m| (m.id.clone(), true))
            .chain(snapshot.sent.iter().map(|m| (m.id.clone(), false)))
            .collect();
        for (id, received) in incoming {
            if !self.order.contains(&id) {
                if received && !first {
                    self.unread.insert(id.clone());
                }
                self.order.push(id);
            }
        }
        if first {
            self.earlier = self.order.len();
        }
        self.snapshot = Some(snapshot);

        self.contact = selected_peer
            .and_then(|id| self.peers().iter().position(|p| p.id == id))
            .unwrap_or_else(|| self.contact.min(self.peers().len().saturating_sub(1)));
        let thread: Vec<String> = self
            .current_thread()
            .iter()
            .map(|r| r.id().to_owned())
            .collect();
        let last = thread.len().saturating_sub(1);
        self.message = match selected_message {
            // Following the newest message keeps following it.
            _ if first || was_at_end => last,
            Some(id) => thread.iter().position(|t| *t == id).unwrap_or(last),
            None => last,
        };

        if self.selected_peer().is_none() {
            self.focus = Focus::Contacts;
        } else if self.focus == Focus::Vault && self.selected_message().is_none() {
            self.focus = Focus::Conversation;
        }
        self.mark_read();
    }
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
            raw: String::new(),
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
            raw: String::new(),
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

    fn refresh(app: &mut App, s: Snapshot) {
        assert!(app.begin_refresh());
        app.finish(Ok(s));
    }

    fn ids(rows: &[Row<'_>]) -> Vec<String> {
        rows.iter().map(|r| r.id().to_owned()).collect()
    }

    #[test]
    fn empty_state_is_navigable() {
        let mut app = loaded(snapshot(&[], &[], &[]));
        for key in [Key::Up, Key::Down, Key::Enter, Key::Tab, Key::Right] {
            assert_eq!(app.key(key), Action::None);
        }
        assert_eq!(app.focus, Focus::Contacts, "nothing to open");
        assert!(app.selected_peer().is_none());
        assert!(app.selected_message().is_none());
    }

    #[test]
    fn no_snapshot_yet_is_empty() {
        let mut app = App::new();
        assert!(app.peers().is_empty() && app.current_thread().is_empty());
        assert_eq!(app.key(Key::Down), Action::None);
    }

    #[test]
    fn panes_open_left_to_right_and_close_right_to_left() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Right);
        assert_eq!(app.focus, Focus::Vault);
        app.key(Key::Esc);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Left);
        assert_eq!(app.focus, Focus::Contacts);
        assert_eq!(app.key(Key::Esc), Action::Quit);
    }

    #[test]
    fn tab_cycles_through_all_panes() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        for focus in [Focus::Conversation, Focus::Vault, Focus::Contacts] {
            app.key(Key::Tab);
            assert_eq!(app.focus, focus);
        }
    }

    #[test]
    fn vault_needs_a_message() {
        let mut app = loaded(snapshot(&["a"], &[], &[]));
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Conversation, "empty thread");
    }

    #[test]
    fn q_quits_from_any_pane() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        app.key(Key::Enter);
        app.key(Key::Enter);
        assert_eq!(app.key(Key::Char('q')), Action::Quit);
    }

    #[test]
    fn threads_hold_both_directions_for_one_peer() {
        let app = loaded(snapshot(
            &["a", "b"],
            &[("m1", "a"), ("m2", "b")],
            &[sent("s1", "a", true), sent("s2", "b", false)],
        ));
        let thread = app.thread("a");
        assert_eq!(ids(&thread), ["m1", "s1"]);
        assert!(!thread[0].outgoing() && thread[1].outgoing());
    }

    #[test]
    fn selecting_a_contact_lands_on_its_newest_message() {
        let mut app = loaded(snapshot(
            &["a", "b"],
            &[("m1", "a"), ("m2", "b"), ("m3", "b")],
            &[],
        ));
        assert_eq!(app.selected_message().unwrap().id(), "m1");
        app.key(Key::Down);
        assert_eq!(app.selected_message().unwrap().id(), "m3");
        app.key(Key::Enter);
        app.key(Key::Up);
        app.key(Key::Up);
        assert_eq!(app.selected_message().unwrap().id(), "m2");
    }

    #[test]
    fn vault_sections_clamp() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        app.key(Key::Tab);
        app.key(Key::Tab);
        app.key(Key::Up);
        assert_eq!(app.section(), Section::Identity);
        for _ in 0..20 {
            app.key(Key::Down);
        }
        assert_eq!(app.section(), Section::Raw);
    }

    #[test]
    fn session_arrivals_follow_earlier_messages_and_are_unread() {
        let mut app = loaded(snapshot(&["a"], &[("m9", "a")], &[]));
        assert_eq!(app.unread_total(), 0, "already stored is not new");
        refresh(
            &mut app,
            snapshot(
                &["a"],
                &[("m1", "a"), ("m9", "a")],
                &[sent("s0", "a", false)],
            ),
        );
        assert_eq!(ids(&app.thread("a")), ["m9", "m1", "s0"]);
        assert!(app.is_earlier("m9") && !app.is_earlier("m1"));
        assert_eq!(app.unread_with("a"), 1, "own sends are never unread");
        assert_eq!(app.unread_total(), 1);
    }

    #[test]
    fn opening_a_conversation_reads_it() {
        let mut app = loaded(snapshot(&["a", "b"], &[], &[]));
        refresh(
            &mut app,
            snapshot(&["a", "b"], &[("m1", "a"), ("m2", "b")], &[]),
        );
        assert_eq!(app.unread_total(), 2);
        app.key(Key::Enter);
        assert_eq!(app.unread_with("a"), 0);
        assert_eq!(app.unread_with("b"), 1);
    }

    #[test]
    fn arrivals_in_the_open_conversation_are_read_at_once() {
        let mut app = loaded(snapshot(&["a"], &[], &[]));
        app.key(Key::Enter);
        refresh(&mut app, snapshot(&["a"], &[("m1", "a")], &[]));
        assert_eq!(app.unread_total(), 0);
        assert_eq!(app.selected_message().unwrap().id(), "m1");
    }

    #[test]
    fn awaiting_ack_counts_only_unacked_sends() {
        let app = loaded(snapshot(
            &["a", "b"],
            &[("m1", "a")],
            &[
                sent("s1", "a", true),
                sent("s2", "a", false),
                sent("s3", "b", false),
            ],
        ));
        assert_eq!(app.awaiting_with("a"), 1);
        assert_eq!(app.awaiting_total(), 2);
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
        assert_eq!(app.thread("a").len(), 1);
    }

    #[test]
    fn unreachable_relay_still_shows_local_state() {
        let mut s = snapshot(&["a"], &[("m1", "a")], &[sent("s1", "a", true)]);
        s.sync = Err("connection refused".into());
        let app = loaded(s);
        assert_eq!(app.relay_ok, Some(false));
        assert!(app.status.contains("relay unreachable"));
        assert_eq!(app.thread("a").len(), 2);
        assert!(
            !app.thread("a")[1].awaiting_ack(),
            "ACK is from the snapshot"
        );
    }

    #[test]
    fn repeated_refreshes_do_not_duplicate_rows() {
        let s = || snapshot(&["a"], &[("m1", "a")], &[sent("s1", "a", true)]);
        let mut app = loaded(s());
        for _ in 0..3 {
            refresh(&mut app, s());
        }
        assert_eq!(app.thread("a").len(), 2);
        assert_eq!(app.unread_total(), 0);
    }

    #[test]
    fn refresh_keeps_selected_contact_by_id() {
        let mut app = loaded(snapshot(&["b", "c"], &[], &[]));
        app.key(Key::Down);
        refresh(&mut app, snapshot(&["a", "b", "c"], &[], &[]));
        assert_eq!(app.selected_peer().unwrap().id, "c");
    }

    #[test]
    fn refresh_keeps_a_message_picked_mid_thread() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a"), ("m2", "a")], &[]));
        app.key(Key::Enter);
        app.key(Key::Up);
        refresh(
            &mut app,
            snapshot(&["a"], &[("m1", "a"), ("m2", "a"), ("m3", "a")], &[]),
        );
        assert_eq!(app.selected_message().unwrap().id(), "m1");
    }

    #[test]
    fn refresh_clamps_when_selection_disappears() {
        let mut app = loaded(snapshot(&["a", "b", "c"], &[], &[]));
        app.key(Key::Down);
        app.key(Key::Down);
        app.key(Key::Enter);
        refresh(&mut app, snapshot(&["a"], &[], &[]));
        assert_eq!(app.selected_peer().unwrap().id, "a");
        refresh(&mut app, snapshot(&[], &[], &[]));
        assert_eq!(app.contact, 0);
        assert_eq!(app.focus, Focus::Contacts, "nothing left to look at");
    }
}
