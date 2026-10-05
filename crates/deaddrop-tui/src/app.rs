//! UI state, kept apart from network state. Pure: no terminal, no I/O.
//!
//! Network state arrives only as a [`Snapshot`] from a refresh. Everything
//! else here belongs to the UI: focus, selection, which vault section is
//! open, the order messages were first seen in, and which arrivals have not
//! been looked at yet. None of it is protocol state.
//!
//! The compose draft lives here too. Sending is not: the app only asks for
//! a send with [`Action::Send`]; the caller runs `Shell::send` and reports
//! back through [`App::finish_send`].

use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

use crate::composer;

#[path = "tasks.rs"]
mod tasks;
use crate::snapshot::{Peer, Received, Sent, Snapshot};
use deaddrop_room::{Kind, RoomConfig, RoomMessage};
pub use tasks::{Note, Planning, TaskRequest};

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
    Backspace,
    /// Remove the character after the composer cursor.
    Delete,
    /// A real line break in the draft (Shift+Enter or Ctrl+J). Never sends.
    Newline,
    Char(char),
    /// Mouse wheel over the conversation: a few rows.
    WheelUp,
    WheelDown,
    /// Ctrl+Up / Ctrl+Down: about half the visible conversation.
    ScrollUp,
    ScrollDown,
    /// Optional, where the keyboard has them.
    PageUp,
    PageDown,
    Home,
    End,
    /// Left click on the nth contact: open that conversation.
    ClickContact(usize),
    /// Left click on the nth room: open its stream.
    ClickRoom(usize),
    /// Click on the room's `λ wire` label: show or hide its machine stream.
    ToggleWire,
    /// Left button pressed in the conversation: focus it, start selecting.
    MouseDown {
        column: u16,
        row: u16,
    },
    /// Pointer moved with the left button held.
    MouseDrag {
        column: u16,
        row: u16,
    },
    /// Left button released.
    MouseUp {
        column: u16,
        row: u16,
    },
    /// Left click in the composer: start writing, cursor at the click.
    ComposerClick {
        column: u16,
        row: u16,
    },
    /// Right click in the composer: paste the clipboard at the cursor.
    ComposerRightClick,
    /// Wheel over the contact list: move the contact selection.
    ContactsUp,
    ContactsDown,
    /// Ctrl+C: copy the selection, or quit when there is none.
    Copy,
}

/// One drawn row of message text: which message, which chars of its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRow {
    pub message: String,
    /// `body.chars()[start..end]` is what the row shows.
    pub start: usize,
    pub end: usize,
    /// Screen column of the row's first character.
    pub x: u16,
}

/// A character in a message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextPos {
    pub message: String,
    pub char: usize,
}

/// Selected conversation text, from `anchor` to `head`, both inclusive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub peer: String,
    pub anchor: TextPos,
    pub head: TextPos,
    /// The button is still down.
    pub dragging: bool,
}

/// Rows one wheel notch moves.
pub const WHEEL_ROWS: usize = 3;

/// One message's rows in the drawn conversation, including the rule and
/// author line above it, so blocks tile the conversation top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRows {
    pub id: String,
    pub start: usize,
    pub end: usize,
}

/// The conversation as last drawn: real visual rows, after wrapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationView {
    pub peer: String,
    /// Visible rows.
    pub rows: usize,
    pub blocks: Vec<MessageRows>,
    /// The first drawn row, and what text every row shows (`None`: a rule,
    /// author line or blank), indexed from the top of the conversation.
    pub offset: usize,
    pub text_rows: Vec<Option<TextRow>>,
    /// Screen cells it occupies, for wheel hit-testing.
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl ConversationView {
    pub fn total(&self) -> usize {
        self.blocks.last().map_or(0, |b| b.end)
    }

    pub fn contains(&self, column: u16, row: u16) -> bool {
        (self.x..self.x.saturating_add(self.width)).contains(&column)
            && (self.y..self.y.saturating_add(self.height)).contains(&row)
    }
}

/// The contact rows as last drawn, for clicks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContactsView {
    pub x: u16,
    /// Screen row of the first contact.
    pub y: u16,
    pub width: u16,
    /// Contacts visible, one row each.
    pub rows: u16,
    /// Screen row of the first room, and rooms visible (0: no rooms).
    pub rooms_y: u16,
    pub room_rows: u16,
}

impl ContactsView {
    /// The room at a screen cell, if any.
    pub fn room_at(&self, column: u16, row: u16) -> Option<usize> {
        let inside = (self.x..self.x.saturating_add(self.width)).contains(&column)
            && (self.rooms_y..self.rooms_y.saturating_add(self.room_rows)).contains(&row);
        inside.then(|| usize::from(row - self.rooms_y))
    }

    /// The contact at a screen cell, if any.
    pub fn at(&self, column: u16, row: u16) -> Option<usize> {
        let inside = (self.x..self.x.saturating_add(self.width)).contains(&column)
            && (self.y..self.y.saturating_add(self.rows)).contains(&row);
        inside.then(|| usize::from(row - self.y))
    }
}

/// A clickable label: screen row and columns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Label {
    pub x: u16,
    pub y: u16,
    pub width: u16,
}

impl Label {
    pub fn contains(&self, column: u16, row: u16) -> bool {
        row == self.y && (self.x..self.x.saturating_add(self.width)).contains(&column)
    }
}

/// The composer as last drawn, for clicks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerView {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// Screen column where draft text starts (after the prompt).
    pub text_x: u16,
    /// Columns the draft wraps to.
    pub cols: usize,
    /// First visible row of the laid-out draft.
    pub offset: usize,
}

impl ComposerView {
    pub fn contains(&self, column: u16, row: u16) -> bool {
        (self.x..self.x.saturating_add(self.width)).contains(&column)
            && (self.y..self.y.saturating_add(self.height)).contains(&row)
    }
}

/// What one frame drew that input can point at.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Drawn {
    pub conversation: Option<ConversationView>,
    pub contacts: Option<ContactsView>,
    pub composer: Option<ComposerView>,
    /// The room header's `λ wire` label.
    pub wire_toggle: Option<Label>,
}

/// Where the top visible row is, held by the message it falls in, so the
/// same text stays in view when rows wrap differently or messages arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub message: String,
    pub row: usize,
}

/// One conversation's scroll state, for this session only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Viewport {
    /// `None` follows the bottom.
    pub anchor: Option<Anchor>,
    /// Incoming messages that arrived while not following.
    pub unseen: usize,
    /// The selected message while another conversation is open.
    selected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Refresh,
    /// Run `Shell::send` with this, then call [`App::finish_send`].
    Send(Outgoing),
    /// Fan this room message out: one signed delivery per member, then call
    /// [`App::finish_room_send`].
    SendRoom(RoomOutgoing),
    /// Put this text on the clipboard.
    Copy(String),
    /// Read the clipboard, then hand it to [`App::pasted`].
    ReadClipboard,
    /// Ask the planner for this task, then hand its answer to
    /// [`App::planned`]. Nothing else runs until the plan is validated.
    PlanTask(TaskRequest),
}

/// A message the user asked to send. Not sent until `Shell::send` says so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub to: String,
    pub body: String,
}

/// A room message to deliver to each member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomOutgoing {
    pub room: String,
    /// The logical room message id, shared by every delivery.
    pub id: String,
    /// Every other member: one delivery each.
    pub to: Vec<String>,
    /// The encoded room message, the same for every delivery.
    pub body: String,
    /// `@names` that matched no member, and so asked no one.
    pub unknown: Vec<String>,
}

/// What a Wire command would need from Agent Wire that does not exist yet.
/// Said plainly instead of pretending to run it.
fn wire_gap(command: &deaddrop_wire_command::WireCommand) -> &'static str {
    use deaddrop_wire_command::WireCommand as W;
    match command {
        W::Objective { .. } => "Agent Wire has no objective planner yet",
        W::Task { .. } => "/task::wire runs in a room",
        W::Inspect { .. } => "Agent Wire has no capability resolver yet",
        W::Status { .. } | W::Trace { .. } => "no Agent Wire runtime is connected to this node",
        W::Cancel { .. } => "Agent Wire has no cancellation yet",
    }
}

/// The one in-memory draft, bound to the contact it was started for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compose {
    pub to: String,
    pub draft: String,
    /// Where typing goes: graphemes of `draft` before the cursor.
    pub cursor: usize,
    /// A send is in flight; the draft is frozen until it reports back.
    pub sending: bool,
}

/// Left to right: who, what was said, what it is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Contacts,
    Conversation,
    Vault,
}

/// How long after one sync finishes the next background sync starts, when
/// nothing is awaited. A local mailbox, not a live socket.
pub const AUTO_SYNC: Duration = Duration::from_secs(3);

/// The background sync while a reply is awaited: a room request not yet
/// answered, or your own send in the last [`AWAIT_WINDOW`]. There is no
/// push from the relay; this is the poll, kept short only while it matters.
pub const FAST_SYNC: Duration = Duration::from_millis(300);
pub const AWAIT_WINDOW: Duration = Duration::from_secs(60);

/// An agent asked in a room that has not reported back yet. Truthful only:
/// whether its delivery was acknowledged (receipt), and how long ago you
/// asked. Nothing about what it is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    /// The request's room message id.
    pub request: String,
    pub member: String,
    /// The member acknowledged receipt of the request.
    pub received: bool,
    /// When you sent it, if in this session.
    pub since: Option<Instant>,
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

    /// The message text. For a room message, the text after the room
    /// header (a slice of the signed body); otherwise the whole body.
    pub fn body(&self) -> &'a str {
        let raw = self.raw_body();
        match RoomMessage::decode(raw) {
            Some(Ok(m)) => &raw[raw.len() - m.text.len()..],
            _ => raw,
        }
    }

    /// The body exactly as signed.
    pub fn raw_body(&self) -> &'a str {
        match self {
            Self::Received(m) => &m.body,
            Self::Sent(m) => &m.body,
        }
    }

    /// The room message this is, if it is one (and well formed).
    pub fn room(&self) -> Option<RoomMessage> {
        RoomMessage::decode(self.raw_body()).and_then(Result::ok)
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

#[derive(Debug)]
pub struct App {
    pub snapshot: Option<Snapshot>,
    pub focus: Focus,
    pub contact: usize,
    /// Index into the selected contact's thread.
    pub message: usize,
    pub section: usize,
    /// Sync only. A send in flight is [`Compose::sending`].
    pub activity: Activity,
    /// The running sync was started by the timer, not the user.
    pub quiet: bool,
    pub status: String,
    /// Whether the last refresh reached the relay; `None` before the first.
    pub relay_ok: Option<bool>,
    /// `Some` while composing. Every key goes to the draft then.
    pub compose: Option<Compose>,
    /// The next refresh lands on the newest message, as after a send.
    follow: bool,
    /// When the last sync finished, for the background timer.
    last_sync: Option<Instant>,
    /// Message ids in the order this UI first saw them. V0 envelopes carry
    /// no time, so this is the only order there is.
    order: Vec<String>,
    /// `order[..earlier]` was already stored when the UI started.
    earlier: usize,
    /// Received this session and not looked at yet.
    unread: BTreeSet<String>,
    /// Scroll state per peer id.
    viewports: HashMap<String, Viewport>,
    /// The conversation as last drawn, for scrolling by real rows.
    view: Option<ConversationView>,
    /// A conversation to open once its contact is known (`--open`).
    open_on: Option<String>,
    /// Selected conversation text, for copying.
    selection: Option<Selection>,
    /// The composer as last drawn, for clicks.
    composer_view: Option<ComposerView>,
    /// Rooms this node is in, from its `rooms.json`.
    rooms: Vec<RoomConfig>,
    /// The open room, if a room (not a peer) is open.
    room_open: Option<usize>,
    /// Show the room's machine stream, 0xd34ddr0p::wire, instead of its
    /// messages.
    pub wire_open: bool,
    /// When you last sent anything, for the sync pace.
    sent_at: Option<Instant>,
    /// When each room request of yours went out, this session.
    room_sent_at: HashMap<String, Instant>,
    /// Local task policy: the peers `/task::wire` may ask. Empty asks no one.
    pub task_policy: Vec<String>,
    tasks: tasks::Tasks,
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
            quiet: false,
            status: "waking up".into(),
            relay_ok: None,
            compose: None,
            follow: false,
            last_sync: None,
            order: Vec::new(),
            earlier: 0,
            unread: BTreeSet::new(),
            viewports: HashMap::new(),
            view: None,
            open_on: None,
            selection: None,
            composer_view: None,
            rooms: Vec::new(),
            room_open: None,
            wire_open: false,
            sent_at: None,
            room_sent_at: HashMap::new(),
            task_policy: Vec::new(),
            tasks: tasks::Tasks::default(),
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
            // Room messages belong to their room, not to a direct thread.
            .filter(|r| r.peer() == peer && r.room().is_none())
            .collect();
        rows.sort_by_key(|r| position.get(r.id()).copied().unwrap_or(usize::MAX));
        rows
    }

    /// A room's stream: every room message for `room`, once per logical
    /// message (your own fan-out shows once), in first-seen order. Each
    /// row keeps its real, verified author.
    pub fn room_thread(&self, room: &str) -> Vec<Row<'_>> {
        let position: HashMap<&str, usize> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut rows: Vec<_> = self
            .rows()
            .into_iter()
            .filter(|r| r.room().is_some_and(|m| m.room == room))
            .collect();
        rows.sort_by_key(|r| position.get(r.id()).copied().unwrap_or(usize::MAX));
        let mut seen = std::collections::HashSet::new();
        rows.retain(|r| {
            let author = if r.outgoing() { "" } else { r.peer() };
            let id = r.room().map(|m| m.id).unwrap_or_default();
            seen.insert((author.to_owned(), id))
        });
        rows
    }

    /// For your own room message: how many members acknowledged receipt,
    /// of how many deliveries. Receipt only — not that anyone answered.
    pub fn room_receipts(&self, room_message: &str) -> (usize, usize) {
        let mut total = 0;
        let mut acked = 0;
        for row in self.rows() {
            if let Row::Sent(m) = row
                && row.room().is_some_and(|r| r.id == room_message)
            {
                total += 1;
                if !m.acked_by.is_empty() {
                    acked += 1;
                }
            }
        }
        (acked, total)
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
        if let Some(room) = self.open_room() {
            return self.room_thread(&room.name);
        }
        self.selected_peer()
            .map_or_else(Vec::new, |p| self.thread(&p.id))
    }

    pub fn set_rooms(&mut self, rooms: Vec<RoomConfig>) {
        self.rooms = rooms;
    }

    pub fn rooms(&self) -> &[RoomConfig] {
        &self.rooms
    }

    pub fn open_room(&self) -> Option<&RoomConfig> {
        self.room_open.and_then(|i| self.rooms.get(i))
    }

    /// What is open: `#room`, or a peer's id. Scroll state, selection and
    /// the composer belong to it.
    pub fn channel(&self) -> Option<String> {
        match self.open_room() {
            Some(room) => Some(format!("#{}", room.name)),
            None => self.selected_peer().map(|p| p.id.clone()),
        }
    }

    /// Open a room's stream.
    fn enter_room(&mut self, i: usize) {
        if i >= self.rooms.len() || self.room_open == Some(i) {
            if i < self.rooms.len() {
                self.focus_on(Focus::Conversation);
            }
            return;
        }
        self.remember_selection();
        self.room_open = Some(i);
        self.selection = None;
        self.restore_selection();
        self.focus_on(Focus::Conversation);
    }

    /// Keep the leaving conversation's selected message for when it returns.
    fn remember_selection(&mut self) {
        let leaving = self.channel();
        let selected = self.selected_message().map(|m| m.id().to_owned());
        if let Some(channel) = leaving {
            self.viewports.entry(channel).or_default().selected = selected;
        }
    }

    /// Back in a conversation: its kept selection if it was scrolled away,
    /// else the newest message.
    fn restore_selection(&mut self) {
        let thread: Vec<String> = self
            .current_thread()
            .iter()
            .map(|r| r.id().to_owned())
            .collect();
        let kept = self
            .channel()
            .and_then(|c| self.viewports.get(&c))
            .filter(|v| v.anchor.is_some())
            .and_then(|v| v.selected.clone());
        self.message = kept
            .and_then(|id| thread.iter().position(|t| *t == id))
            .unwrap_or(thread.len().saturating_sub(1));
    }

    /// The left rail: rooms first, then peers. Where the open entry is.
    fn rail_position(&self) -> usize {
        self.room_open.unwrap_or(self.rooms.len() + self.contact)
    }

    fn rail_move(&mut self, delta: isize) {
        let len = self.rooms.len() + self.peers().len();
        if len == 0 {
            return;
        }
        let next = self
            .rail_position()
            .saturating_add_signed(delta)
            .min(len - 1);
        if next < self.rooms.len() {
            self.enter_room(next);
        } else {
            self.leave_room();
            self.select_contact(next - self.rooms.len());
        }
    }

    /// Close the open room, back to the selected peer.
    fn leave_room(&mut self) {
        if self.room_open.is_some() {
            self.remember_selection();
            self.room_open = None;
            self.selection = None;
            self.restore_selection();
        }
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

    pub fn composing(&self) -> bool {
        self.compose.is_some()
    }

    pub fn key(&mut self, key: Key) -> Action {
        use Focus::*;
        // Scrolling reads; it never touches the draft. Home/End stay free for
        // composer editing while a draft is open.
        let page = self.view.as_ref().map_or(1, |v| v.rows.max(1));
        match key {
            Key::WheelUp => return self.scroll_by(-(WHEEL_ROWS as isize)),
            Key::WheelDown => return self.scroll_by(WHEEL_ROWS as isize),
            Key::ScrollUp => return self.scroll_by(-((page / 2).max(1) as isize)),
            Key::ScrollDown => return self.scroll_by((page / 2).max(1) as isize),
            Key::PageUp => return self.scroll_by(-(page.saturating_sub(2).max(1) as isize)),
            Key::PageDown => return self.scroll_by(page.saturating_sub(2).max(1) as isize),
            Key::Home if !self.composing() => return self.scroll_by(isize::MIN / 2),
            Key::End if !self.composing() => return self.scroll_by(isize::MAX / 2),
            _ => {}
        }
        match key {
            Key::ClickContact(i) => {
                // A draft is bound to its contact; never drop typed text.
                if self.compose.as_ref().is_some_and(|c| !c.draft.is_empty()) {
                    self.status = "finish or esc the draft first".into();
                    return Action::None;
                }
                if i < self.peers().len() {
                    self.compose = None;
                    self.leave_room();
                    self.select_contact(i);
                    self.focus_on(Focus::Conversation);
                }
                return Action::None;
            }
            Key::ClickRoom(i) => {
                if self.compose.as_ref().is_some_and(|c| !c.draft.is_empty()) {
                    self.status = "finish or esc the draft first".into();
                    return Action::None;
                }
                if i < self.rooms.len() {
                    self.compose = None;
                    self.enter_room(i);
                }
                return Action::None;
            }
            Key::ToggleWire => {
                if self.open_room().is_some() {
                    self.wire_open = !self.wire_open;
                }
                return Action::None;
            }
            Key::MouseDown { column, row } => {
                if !self.composing() && self.channel().is_some() {
                    self.focus_on(Focus::Conversation);
                }
                let peer = self.channel();
                self.selection = peer
                    .zip(self.text_at(column, row))
                    .map(|(peer, pos)| Selection {
                        peer,
                        anchor: pos.clone(),
                        head: pos,
                        dragging: true,
                    });
                return Action::None;
            }
            Key::MouseDrag { column, row } | Key::MouseUp { column, row } => {
                let pos = self.text_at(column, row);
                let up = matches!(key, Key::MouseUp { .. });
                if let Some(sel) = self.selection.as_mut().filter(|s| s.dragging) {
                    if let Some(pos) = pos {
                        sel.head = pos;
                    }
                    if up {
                        sel.dragging = false;
                        // A click is not a selection.
                        if sel.anchor == sel.head {
                            self.selection = None;
                        }
                    }
                }
                return Action::None;
            }
            Key::ComposerClick { column, row } => {
                self.click_composer(column, row);
                return Action::None;
            }
            // Focus first, then ask for the clipboard; it lands at the
            // cursor when it arrives. Nothing is sent.
            Key::ComposerRightClick => {
                if self.channel().is_none() || self.sending() {
                    return Action::None;
                }
                if !self.composing() {
                    self.start_compose();
                }
                return Action::ReadClipboard;
            }
            Key::ContactsUp | Key::ContactsDown => {
                if self.compose.as_ref().is_some_and(|c| !c.draft.is_empty()) {
                    self.status = "finish or esc the draft first".into();
                    return Action::None;
                }
                self.compose = None;
                self.rail_move(if key == Key::ContactsUp { -1 } else { 1 });
                self.focus = Focus::Contacts;
                return Action::None;
            }
            Key::Copy => {
                return match self.selected_text() {
                    Some(text) if !text.is_empty() => {
                        self.status = format!("copied {} chars", text.chars().count());
                        Action::Copy(text)
                    }
                    _ => Action::Quit,
                };
            }
            Key::Esc if self.selection.is_some() => {
                self.selection = None;
                return Action::None;
            }
            _ => {}
        }
        if self.composing() {
            return self.compose_key(key);
        }
        match (self.focus, key) {
            // The conversation is where you talk: typing there writes.
            (Conversation, Key::Char(c)) if !c.is_control() => {
                self.start_compose();
                return self.compose_key(key);
            }
            (_, Key::Char('q')) => return Action::Quit,
            (_, Key::Char('r')) => return Action::Refresh,
            (Contacts, Key::Char('i')) => self.start_compose(),
            (Contacts, Key::Esc) => return Action::Quit,

            (Contacts, Key::Up | Key::Char('k')) => self.pick_contact(-1),
            (Contacts, Key::Down | Key::Char('j')) => self.pick_contact(1),
            (Contacts, Key::Enter | Key::Right | Key::Tab | Key::Char('l')) => {
                if self.selected_peer().is_some() {
                    self.focus_on(Conversation);
                }
            }

            (Conversation, Key::Up) => self.pick_message(-1),
            (Conversation, Key::Down) => self.pick_message(1),
            (Conversation, Key::Enter | Key::Right | Key::Tab) => {
                if self.selected_message().is_some() {
                    self.focus_on(Vault);
                }
            }
            (Conversation, Key::Esc | Key::Left | Key::BackTab) => self.focus_on(Contacts),

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

    /// Writing happens in the conversation, so composing opens it.
    fn start_compose(&mut self) {
        if let Some(to) = self.channel() {
            self.compose = Some(Compose {
                to,
                draft: String::new(),
                cursor: 0,
                sending: false,
            });
            self.focus_on(Focus::Conversation);
        }
    }

    pub fn sending(&self) -> bool {
        self.compose.as_ref().is_some_and(|c| c.sending)
    }

    fn compose_key(&mut self, key: Key) -> Action {
        let Some(compose) = &mut self.compose else {
            return Action::None;
        };
        if compose.sending {
            return Action::None;
        }
        match key {
            Key::Esc => self.compose = None,
            // Editing happens at the cursor.
            Key::Backspace => composer::backspace(&mut compose.draft, &mut compose.cursor),
            Key::Delete => composer::delete(&mut compose.draft, compose.cursor),
            Key::Left => composer::left(&mut compose.cursor),
            Key::Right => composer::right(&compose.draft, &mut compose.cursor),
            Key::Char(c) if !c.is_control() => composer::insert(
                &mut compose.draft,
                &mut compose.cursor,
                c.encode_utf8(&mut [0; 4]),
            ),
            Key::Newline => composer::insert(&mut compose.draft, &mut compose.cursor, "\n"),
            Key::Enter if compose.draft.trim().is_empty() => {
                self.status = "nothing to send".into();
            }
            // A sync in flight is fine: the caller runs jobs one at a time.
            Key::Enter => {
                // Control syntax first, deterministically: a valid Wire command
                // is intent, never chat; an invalid one is an error, never chat.
                match deaddrop_wire_command::parse(&compose.draft) {
                    deaddrop_wire_command::Parsed::Command(
                        deaddrop_wire_command::WireCommand::Task { payload, dry_run },
                    ) => {
                        return self.start_task(payload, dry_run);
                    }
                    deaddrop_wire_command::Parsed::Command(command) => {
                        self.status = format!(
                            "wire: {} accepted as intent · not run: {}",
                            command.name(),
                            wire_gap(&command)
                        );
                        return Action::None;
                    }
                    deaddrop_wire_command::Parsed::Invalid(error) => {
                        self.status = error.to_string();
                        return Action::None;
                    }
                    deaddrop_wire_command::Parsed::Ordinary => {}
                }
                if let Some(name) = compose.to.strip_prefix('#') {
                    let me = self.snapshot.as_ref().map(|s| s.node.id.clone());
                    let Some(room) = self.rooms.iter().find(|r| r.name == name) else {
                        self.status = format!("not in room #{name}");
                        return Action::None;
                    };
                    let others: Vec<String> = room
                        .members
                        .iter()
                        .filter(|m| Some(*m) != me.as_ref())
                        .cloned()
                        .collect();
                    // Only explicit mentions of members ask anyone.
                    let (asked, unknown) = deaddrop_room::mentions(&compose.draft, &others);
                    let id = deaddrop_protocol::MessageId::generate().to_string();
                    let body = RoomMessage {
                        room: room.name.clone(),
                        id: id.clone(),
                        kind: if asked.is_empty() {
                            Kind::Message
                        } else {
                            Kind::Request
                        },
                        hop: 0,
                        mentions: asked,
                        reply_to: None,
                        status: None,
                        text: compose.draft.clone(),
                    }
                    .encode();
                    compose.sending = true;
                    return Action::SendRoom(RoomOutgoing {
                        room: room.name.clone(),
                        id,
                        to: others,
                        body,
                        unknown,
                    });
                }
                compose.sending = true;
                let outgoing = Outgoing {
                    to: compose.to.clone(),
                    body: compose.draft.clone(),
                };
                return Action::Send(outgoing);
            }
            _ => {}
        }
        Action::None
    }

    /// Apply a finished `Shell::send`. Success means the shell signed,
    /// published, and stored it — not that anyone has it. Compose stays
    /// open for the next message. A failure keeps the draft so nothing typed
    /// is lost.
    pub fn finish_send(&mut self, result: Result<String, String>) {
        let Some(compose) = &mut self.compose else {
            return;
        };
        compose.sending = false;
        match result {
            Ok(_) => {
                self.sent_at = Some(Instant::now());
                self.status = format!("sent to {}", crate::ui::name(&compose.to).0);
                compose.draft.clear();
                compose.cursor = 0;
                self.follow = true;
            }
            Err(error) => self.status = format!("send failed: {error}"),
        }
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
        self.leave_room();
        let len = self.peers().len();
        if len == 0 {
            return;
        }
        self.select_contact(self.contact.saturating_add_signed(delta).min(len - 1));
    }

    fn select_contact(&mut self, next: usize) {
        if next == self.contact || next >= self.peers().len() {
            return;
        }
        self.selection = None;
        // Each conversation keeps its place and its selection.
        self.remember_selection();
        self.contact = next;
        self.restore_selection();
    }

    fn pick_message(&mut self, delta: isize) {
        let len = self.current_thread().len();
        self.message = match len {
            0 => 0,
            _ => self.message.saturating_add_signed(delta).min(len - 1),
        };
        self.reveal_selected();
    }

    /// The text position under a screen cell in the drawn conversation. A
    /// row with no text (a rule or author line) resolves to the nearest text
    /// above it, else below; rows past the pane's edges are clamped.
    fn text_at(&self, column: u16, row: u16) -> Option<TextPos> {
        let view = self.current_view()?;
        let last = view.y.checked_add(view.height)?.checked_sub(1)?;
        let row = row.clamp(view.y, last);
        let at = view.offset + usize::from(row - view.y);
        let rows = &view.text_rows;
        let (text, end_of_row) = match rows.get(at).cloned().flatten() {
            Some(t) => (t, false),
            None => match rows[..at.min(rows.len())].iter().rev().flatten().next() {
                Some(t) => (t.clone(), true),
                None => (rows.get(at..)?.iter().flatten().next()?.clone(), false),
            },
        };
        let body: Vec<char> = self
            .current_thread()
            .iter()
            .find(|r| r.id() == text.message)?
            .body()
            .chars()
            .collect();
        let last_char = text.end.saturating_sub(1).max(text.start);
        let char = if end_of_row {
            last_char
        } else {
            let mut x = text.x;
            let mut found = last_char;
            for (i, c) in body.iter().enumerate().take(text.end).skip(text.start) {
                let w = crate::ui::width(c.encode_utf8(&mut [0; 4])) as u16;
                if column < x.saturating_add(w) {
                    found = i;
                    break;
                }
                x = x.saturating_add(w);
            }
            if column < text.x { text.start } else { found }
        };
        Some(TextPos {
            message: text.message,
            char,
        })
    }

    /// The selection's ends in conversation order: (thread index, char).
    fn selection_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        let sel = self.selection.as_ref()?;
        if self.channel()? != sel.peer {
            return None;
        }
        let thread = self.current_thread();
        let index = |p: &TextPos| thread.iter().position(|r| r.id() == p.message);
        let a = (index(&sel.anchor)?, sel.anchor.char);
        let b = (index(&sel.head)?, sel.head.char);
        Some(if a <= b { (a, b) } else { (b, a) })
    }

    /// The selected chars of one message, inclusive, for highlighting.
    pub fn selected_chars(&self, message: &str) -> Option<(usize, usize)> {
        let ((ia, ca), (ib, cb)) = self.selection_bounds()?;
        let thread = self.current_thread();
        let i = thread.iter().position(|r| r.id() == message)?;
        if i < ia || i > ib {
            return None;
        }
        let from = if i == ia { ca } else { 0 };
        let to = if i == ib { cb } else { usize::MAX };
        Some((from, to))
    }

    /// The selected text, exactly as written: soft wraps are not newlines,
    /// real newlines stay, nothing from the screen layout is included.
    /// Parts from several messages are joined by a newline.
    pub fn selected_text(&self) -> Option<String> {
        let ((ia, _), (ib, _)) = self.selection_bounds()?;
        let thread = self.current_thread();
        let mut parts = Vec::new();
        for row in &thread[ia..=ib] {
            let chars: Vec<char> = row.body().chars().collect();
            let Some((from, to)) = self.selected_chars(row.id()) else {
                continue;
            };
            if chars.is_empty() || from >= chars.len() {
                continue;
            }
            let to = to.min(chars.len() - 1);
            parts.push(chars[from..=to].iter().collect::<String>());
        }
        Some(parts.join("\n"))
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Record the composer as just drawn.
    pub fn observe_composer(&mut self, view: Option<ComposerView>) {
        self.composer_view = view;
    }

    /// A click in the composer starts writing to the open contact, and puts
    /// the cursor where the click landed in the draft. Sends nothing.
    fn click_composer(&mut self, column: u16, row: u16) {
        if self.channel().is_none() || self.sending() {
            return;
        }
        if !self.composing() {
            self.start_compose();
        }
        let Some(view) = self.composer_view.clone() else {
            return;
        };
        let Some(compose) = self.compose.as_mut() else {
            return;
        };
        let line = view.offset + usize::from(row.saturating_sub(view.y));
        let col = usize::from(column.saturating_sub(view.text_x));
        compose.cursor = composer::cursor_at(&compose.draft, view.cols, line, col);
    }

    /// Text pasted into the terminal goes into the draft at the cursor, as
    /// typed text would — newlines included — and is never sent: only Enter
    /// sends. Opens the composer if a contact is open.
    pub fn paste(&mut self, text: &str) {
        if self.channel().is_none() || self.sending() {
            return;
        }
        if !self.composing() {
            self.start_compose();
        }
        let Some(compose) = self.compose.as_mut() else {
            return;
        };
        let clean: String = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\t', "    ")
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        composer::insert(&mut compose.draft, &mut compose.cursor, &clean);
    }

    /// How a room fan-out went: one result per member. Any delivery that
    /// worked makes the message dropped; failures are said, not hidden, and
    /// the deliveries that worked stand.
    pub fn finish_room_send(
        &mut self,
        outgoing: &RoomOutgoing,
        results: Vec<(String, Result<String, String>)>,
    ) {
        let (room, unknown) = (outgoing.room.as_str(), outgoing.unknown.as_slice());
        if results.iter().any(|(_, r)| r.is_ok()) {
            self.room_sent_at
                .insert(outgoing.id.clone(), Instant::now());
        }
        let total = results.len();
        let ok = results.iter().filter(|(_, r)| r.is_ok()).count();
        let first_ok = results.iter().find_map(|(_, r)| r.as_ref().ok().cloned());
        match first_ok {
            Some(id) => self.finish_send(Ok(id)),
            None => {
                let why = results
                    .iter()
                    .find_map(|(_, r)| r.as_ref().err().cloned())
                    .unwrap_or_else(|| "no other members".into());
                self.finish_send(Err(why));
                return;
            }
        }
        let mut status = format!("dropped to #{room} · relay took {ok}/{total}");
        if !unknown.is_empty() {
            let names: Vec<String> = unknown.iter().map(|n| format!("@{n}")).collect();
            status.push_str(&format!(" · {} not in room, not asked", names.join(" ")));
        }
        self.status = status;
    }

    /// Is a reply awaited? Then sync at [`FAST_SYNC`].
    pub fn awaiting(&self, now: Instant) -> bool {
        self.sent_at
            .is_some_and(|t| now.saturating_duration_since(t) < AWAIT_WINDOW)
            || !self.pending().is_empty()
            || self.task_running()
    }

    /// Your room requests' asked members that have not reported back.
    pub fn pending(&self) -> Vec<Pending> {
        let rows = self.rows();
        // Who has reported on which request: (member, request id).
        let answered: std::collections::HashSet<(String, String)> = rows
            .iter()
            .filter(|r| !r.outgoing())
            .filter_map(|r| {
                let m = r.room()?;
                (m.kind == Kind::Report)
                    .then(|| (r.peer().to_owned(), m.reply_to.clone().unwrap_or_default()))
            })
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut pending = Vec::new();
        for row in &rows {
            let Row::Sent(_) = row else { continue };
            let Some(m) = row.room().filter(|m| m.kind == Kind::Request) else {
                continue;
            };
            if !seen.insert(m.id.clone()) {
                continue;
            }
            for member in &m.mentions {
                if answered.contains(&(member.clone(), m.id.clone())) {
                    continue;
                }
                let received = rows.iter().any(|r| match r {
                    Row::Sent(s) => {
                        s.to == *member
                            && !s.acked_by.is_empty()
                            && r.room().is_some_and(|x| x.id == m.id)
                    }
                    Row::Received(_) => false,
                });
                pending.push(Pending {
                    request: m.id.clone(),
                    member: member.clone(),
                    received,
                    since: self.room_sent_at.get(&m.id).copied(),
                });
            }
        }
        pending
    }

    /// What a clipboard read returned, after a right click. Text goes in at
    /// the cursor like any paste; an empty clipboard changes nothing; a
    /// failure changes nothing but the status line.
    pub fn pasted(&mut self, read: Result<Option<String>, String>) {
        match read {
            Ok(Some(text)) => self.paste(&text),
            Ok(None) => {}
            Err(error) => self.status = format!("clipboard: {error}"),
        }
    }

    /// Open `peer`'s conversation as soon as it is known.
    pub fn open_on(&mut self, peer: impl Into<String>) {
        self.open_on = Some(peer.into());
    }

    /// This peer's scroll state.
    pub fn viewport(&self, peer: &str) -> Viewport {
        self.viewports.get(peer).cloned().unwrap_or_default()
    }

    fn follow_bottom(&mut self, peer: &str) {
        let viewport = self.viewports.entry(peer.to_owned()).or_default();
        viewport.anchor = None;
        viewport.unseen = 0;
    }

    /// The first visible row for `peer`, given the drawn `blocks` and
    /// `rows` visible. Following, or anchored past the end, is the bottom.
    pub fn scroll_offset(&self, peer: &str, rows: usize, blocks: &[MessageRows]) -> usize {
        let total = blocks.last().map_or(0, |b| b.end);
        let bottom = total.saturating_sub(rows);
        let anchor = self.viewports.get(peer).and_then(|v| v.anchor.as_ref());
        match anchor {
            None => bottom,
            Some(a) => blocks
                .iter()
                .find(|b| b.id == a.message)
                .map_or(bottom, |b| {
                    (b.start + a.row.min(b.end.saturating_sub(b.start + 1))).min(bottom)
                }),
        }
    }

    /// Record the conversation as just drawn. An anchor that now reaches
    /// the bottom (the window grew, say) becomes following again.
    pub fn observe(&mut self, view: Option<ConversationView>) {
        if let Some(v) = &view {
            let anchored = self
                .viewports
                .get(&v.peer)
                .is_some_and(|p| p.anchor.is_some());
            let bottom = v.total().saturating_sub(v.rows);
            if anchored && self.scroll_offset(&v.peer, v.rows, &v.blocks) >= bottom {
                self.follow_bottom(&v.peer.clone());
            }
        }
        self.view = view;
    }

    /// The conversation as last drawn.
    pub fn view(&self) -> Option<&ConversationView> {
        self.view.as_ref()
    }

    /// The drawn view, if it shows the selected peer's conversation.
    fn current_view(&self) -> Option<ConversationView> {
        let key = self.view_key()?;
        self.view.clone().filter(|v| v.peer == key)
    }

    /// Whose scroll state the right pane shows: the open channel's, or its
    /// wire stream's (`#room::wire`), which is kept apart.
    pub fn view_key(&self) -> Option<String> {
        let channel = self.channel()?;
        Some(if self.wire_open && self.open_room().is_some() {
            format!("{channel}::wire")
        } else {
            channel
        })
    }

    fn move_to(&mut self, view: &ConversationView, offset: usize) {
        let bottom = view.total().saturating_sub(view.rows);
        if offset >= bottom {
            self.follow_bottom(&view.peer);
            return;
        }
        let anchor = view
            .blocks
            .iter()
            .find(|b| (b.start..b.end).contains(&offset))
            .map(|b| Anchor {
                message: b.id.clone(),
                row: offset - b.start,
            });
        self.viewports.entry(view.peer.clone()).or_default().anchor = anchor;
    }

    /// Move the conversation by `delta` rows, clamped to its ends.
    fn scroll_by(&mut self, delta: isize) -> Action {
        let Some(view) = self.current_view() else {
            return Action::None;
        };
        let bottom = view.total().saturating_sub(view.rows);
        let now = self.scroll_offset(&view.peer, view.rows, &view.blocks);
        let next = now.saturating_add_signed(delta).min(bottom);
        self.move_to(&view, next);
        self.keep_selection_in_view(&view, next, delta < 0);
        Action::None
    }

    /// After scrolling, a selection that left the view moves to the nearest
    /// message still in it, so selection and viewport never disagree.
    fn keep_selection_in_view(&mut self, view: &ConversationView, offset: usize, up: bool) {
        let visible = |b: &MessageRows| b.end > offset && b.start < offset + view.rows;
        let selected = self.selected_message().map(|m| m.id().to_owned());
        if view
            .blocks
            .iter()
            .any(|b| Some(&b.id) == selected.as_ref() && visible(b))
        {
            return;
        }
        let mut in_view = view.blocks.iter().filter(|b| visible(b));
        let pick = if up {
            in_view.next_back()
        } else {
            in_view.next()
        };
        if let Some(block) = pick {
            let thread = self.current_thread();
            if let Some(i) = thread.iter().position(|r| r.id() == block.id) {
                self.message = i;
            }
        }
    }

    /// Scroll just enough to show the selected message.
    fn reveal_selected(&mut self) {
        let Some(view) = self.current_view() else {
            return;
        };
        let Some(id) = self.selected_message().map(|m| m.id().to_owned()) else {
            return;
        };
        let Some(block) = view.blocks.iter().find(|b| b.id == id).cloned() else {
            return;
        };
        let now = self.scroll_offset(&view.peer, view.rows, &view.blocks);
        let next = if block.start < now {
            block.start
        } else if block.end > now + view.rows {
            block
                .end
                .saturating_sub(view.rows)
                .max(block.start.min(block.end))
        } else {
            return;
        };
        self.move_to(&view, next);
    }

    /// Start a refresh unless one is already running.
    pub fn begin_refresh(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        self.activity = Activity::Busy("syncing");
        self.quiet = false;
        true
    }

    /// Start a background sync if one is due: none running, and [`AUTO_SYNC`]
    /// has passed since the last one finished.
    pub fn begin_auto_sync(&mut self, now: Instant) -> bool {
        let pace = if self.awaiting(now) {
            FAST_SYNC
        } else {
            AUTO_SYNC
        };
        let due = self
            .last_sync
            .is_none_or(|last| now.saturating_duration_since(last) >= pace);
        if !due || !self.begin_refresh() {
            return false;
        }
        self.quiet = true;
        true
    }

    /// Note when a sync finished, for [`App::begin_auto_sync`].
    pub fn synced_at(&mut self, now: Instant) {
        self.last_sync = Some(now);
    }

    pub fn progress(&mut self, label: &'static str) {
        if self.busy() {
            self.activity = Activity::Busy(label);
        }
    }

    /// Apply a finished refresh. A failure keeps the previous snapshot.
    pub fn finish(&mut self, result: Result<Snapshot, String>) {
        self.activity = Activity::Idle;
        let quiet = std::mem::take(&mut self.quiet);
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.status = format!("refresh failed: {error}");
                return;
            }
        };
        let first = self.snapshot.is_none();
        let selected_peer = self.selected_peer().map(|p| p.id.clone());
        let channel = self.channel();
        let selected_message = self.selected_message().map(|m| m.id().to_owned());
        // After your own send the conversation snaps to the bottom. Otherwise
        // the selection moves to a new message only if it was on the newest
        // and the view follows the bottom; a message picked further up, or a
        // reader scrolled away, stays put.
        let sent = std::mem::take(&mut self.follow);
        if sent && let Some(open) = channel.clone() {
            self.follow_bottom(&open);
        }
        let at_end = self.message + 1 >= self.current_thread().len();
        let following = sent
            || (at_end
                && channel
                    .as_ref()
                    .is_none_or(|c| self.viewport(c).anchor.is_none()));

        let relay_ok = Some(snapshot.sync.is_ok());
        let nothing_new = match &snapshot.sync {
            Ok(s) => s.received + s.acknowledged + s.rejected == 0,
            Err(_) => true,
        };
        // A background sync that changed nothing leaves the status line alone,
        // so a failure message stays readable.
        let keep_status = quiet && nothing_new && relay_ok == self.relay_ok;
        self.relay_ok = relay_ok;
        let status = match &snapshot.sync {
            Ok(sync) => format!(
                "synced · {} new · {} ack · {} rejected",
                sync.received, sync.acknowledged, sync.rejected
            ),
            Err(error) => format!("relay unreachable · showing local state · {error}"),
        };
        if !keep_status {
            self.status = status;
        }

        // New arrivals count against the conversation they show in: a room
        // message against its room, anything else against its sender.
        let incoming: Vec<(String, Option<String>)> = snapshot
            .inbox
            .iter()
            .map(|m| {
                let channel = match RoomMessage::decode(&m.body) {
                    Some(Ok(room)) => format!("#{}", room.room),
                    _ => m.from.clone(),
                };
                (m.id.clone(), Some(channel))
            })
            .chain(snapshot.sent.iter().map(|m| (m.id.clone(), None)))
            .collect();
        for (id, from) in incoming {
            if !self.order.contains(&id) {
                if let Some(from) = from.filter(|_| !first) {
                    self.unread.insert(id.clone());
                    // Arrived below a reader who scrolled away: count it.
                    if let Some(v) = self.viewports.get_mut(&from)
                        && v.anchor.is_some()
                    {
                        v.unseen += 1;
                    }
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
            _ if first || following => last,
            Some(id) => thread.iter().position(|t| *t == id).unwrap_or(last),
            None => last,
        };

        if let Some(peer) = self.open_on.take() {
            if let Some(name) = peer.strip_prefix('#') {
                match self.rooms.iter().position(|r| r.name == name) {
                    Some(i) => self.enter_room(i),
                    None => self.status = format!("not in room {peer}"),
                }
            } else {
                match self.peers().iter().position(|p| p.id == peer) {
                    Some(i) => {
                        self.select_contact(i);
                        self.focus = Focus::Conversation;
                    }
                    None => self.status = format!("no contact {peer}"),
                }
            }
        }
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
    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            assert_eq!(app.key(Key::Char(c)), Action::None);
        }
    }

    fn draft(app: &App) -> &str {
        &app.compose.as_ref().expect("composing").draft
    }

    /// Enter on a composed draft, expecting a send request.
    fn enter(app: &mut App) -> Outgoing {
        match app.key(Key::Enter) {
            Action::Send(outgoing) => outgoing,
            other => panic!("expected a send, got {other:?}"),
        }
    }

    fn in_conversation(peers: &[&str], inbox: &[(&str, &str)]) -> App {
        let mut app = loaded(snapshot(peers, inbox, &[]));
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Conversation);
        app
    }

    #[test]
    fn typing_in_the_conversation_starts_a_draft() {
        let mut app = in_conversation(&["a"], &[("m1", "a")]);
        assert_eq!(app.key(Key::Char('h')), Action::None);
        assert_eq!(draft(&app), "h");
        assert_eq!(app.compose.as_ref().unwrap().to, "a");
        assert_eq!(app.focus, Focus::Conversation);
    }

    #[test]
    fn i_writes_from_contacts_and_opens_the_conversation() {
        let mut app = loaded(snapshot(&["a", "b"], &[], &[]));
        app.key(Key::Down);
        assert_eq!(app.key(Key::Char('i')), Action::None);
        let compose = app.compose.as_ref().unwrap();
        assert_eq!((compose.to.as_str(), compose.draft.as_str()), ("b", ""));
        assert_eq!(app.focus, Focus::Conversation);
    }

    #[test]
    fn compose_needs_a_contact() {
        let mut app = loaded(snapshot(&[], &[], &[]));
        app.key(Key::Char('i'));
        assert!(!app.composing());
        let mut app = App::new();
        app.key(Key::Char('i'));
        assert!(!app.composing(), "no snapshot yet");
    }

    #[test]
    fn contacts_keys_stay_commands() {
        let mut app = loaded(snapshot(&["a", "b"], &[], &[]));
        app.key(Key::Char('j'));
        assert_eq!(app.selected_peer().unwrap().id, "b");
        assert!(!app.composing());
        assert_eq!(app.key(Key::Char('r')), Action::Refresh);
        assert_eq!(app.key(Key::Char('q')), Action::Quit);
    }

    #[test]
    fn compose_takes_printable_unicode_and_backspace() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "привет 🌼 é");
        assert_eq!(draft(&app), "привет 🌼 é");
        app.key(Key::Backspace);
        app.key(Key::Backspace);
        assert_eq!(draft(&app), "привет 🌼");
        app.key(Key::Char('\u{7}'));
        assert_eq!(draft(&app), "привет 🌼", "control chars are not text");
        for _ in 0..20 {
            app.key(Key::Backspace);
        }
        assert_eq!(draft(&app), "", "backspace on empty is harmless");
        assert!(app.composing());
    }

    #[test]
    fn esc_leaves_compose_one_level_at_a_time() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "never mind");
        assert_eq!(app.key(Key::Esc), Action::None);
        assert!(!app.composing());
        assert_eq!(app.focus, Focus::Conversation, "only compose closed");
        app.key(Key::Char('x'));
        assert_eq!(draft(&app), "x", "a cancelled draft is gone");
        app.key(Key::Esc);
        app.key(Key::Esc);
        assert_eq!(app.focus, Focus::Contacts);
    }

    #[test]
    fn command_and_navigation_keys_are_text_while_composing() {
        let mut app = in_conversation(&["a", "b"], &[("m1", "a"), ("m2", "a")]);
        let (contact, message) = (app.contact, app.message);
        typed(&mut app, "qrjkhli");
        for key in [
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Tab,
            Key::BackTab,
        ] {
            assert_eq!(app.key(key), Action::None);
        }
        assert_eq!(draft(&app), "qrjkhli");
        assert_eq!((app.contact, app.message), (contact, message));
        assert_eq!(app.focus, Focus::Conversation);
    }

    #[test]
    fn blank_drafts_are_not_sent() {
        let mut app = in_conversation(&["a"], &[]);
        app.key(Key::Char('i'));
        app.key(Key::Backspace);
        assert_eq!(app.key(Key::Enter), Action::None);
        typed(&mut app, "  \u{3000} ");
        assert_eq!(app.key(Key::Enter), Action::None);
        assert!(app.composing() && !app.sending());
        assert_eq!(draft(&app), "  \u{3000} ");
    }

    #[test]
    fn enter_asks_to_send_to_the_compose_contact() {
        let mut app = loaded(snapshot(&["a", "b"], &[], &[]));
        app.key(Key::Down);
        app.key(Key::Enter);
        typed(&mut app, "hello b");
        let outgoing = enter(&mut app);
        assert_eq!(outgoing.to, "b");
        assert_eq!(outgoing.body, "hello b");
        assert!(app.sending());
        assert_eq!(app.key(Key::Enter), Action::None, "one send at a time");
        app.key(Key::Char('x'));
        app.key(Key::Esc);
        assert_eq!(draft(&app), "hello b", "frozen while sending");
    }

    #[test]
    fn send_does_not_wait_for_a_sync() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "hi");
        assert!(app.begin_auto_sync(Instant::now()));
        enter(&mut app);
        assert!(app.sending() && app.busy());
    }

    #[test]
    fn successful_send_stays_composing_for_the_next_message() {
        let mut app = loaded(snapshot(&["a", "b"], &[("m1", "b"), ("m2", "b")], &[]));
        app.key(Key::Down);
        app.key(Key::Enter);
        app.key(Key::Up);
        typed(&mut app, "first");
        enter(&mut app);
        app.finish_send(Ok("s1".into()));
        assert_eq!(draft(&app), "", "draft cleared");
        assert!(!app.sending());
        assert!(app.status.contains("sent"));
        assert!(!app.status.contains("ack"), "sent is not acknowledged");

        refresh(
            &mut app,
            snapshot(
                &["a", "b"],
                &[("m1", "b"), ("m2", "b")],
                &[sent("s1", "b", false)],
            ),
        );
        assert_eq!(app.selected_peer().unwrap().id, "b");
        let row = app.selected_message().unwrap();
        assert_eq!(row.id(), "s1");
        assert!(row.outgoing() && row.awaiting_ack());
        assert!(app.thread("a").is_empty());

        typed(&mut app, "second");
        let outgoing = enter(&mut app);
        assert_eq!(
            (outgoing.to.as_str(), outgoing.body.as_str()),
            ("b", "second")
        );
    }

    #[test]
    fn failed_send_keeps_draft_and_compose_mode() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "hello");
        enter(&mut app);
        app.finish_send(Err("relay unreachable".into()));
        assert_eq!(draft(&app), "hello");
        assert!(!app.sending());
        assert!(app.status.contains("send failed") && app.status.contains("relay unreachable"));
        typed(&mut app, "!");
        assert_eq!(enter(&mut app).body, "hello!");
    }

    #[test]
    fn auto_sync_waits_its_turn_and_never_overlaps() {
        let t0 = Instant::now();
        let mut app = App::new();
        assert!(app.begin_auto_sync(t0), "first sync is due at once");
        assert!(!app.begin_auto_sync(t0 + AUTO_SYNC * 2), "one in flight");
        assert!(!app.begin_refresh(), "not even a forced one");
        app.finish(Ok(snapshot(&["a"], &[], &[])));
        app.synced_at(t0);
        assert!(!app.begin_auto_sync(t0 + AUTO_SYNC / 2), "not due yet");
        assert!(app.begin_auto_sync(t0 + AUTO_SYNC));
        assert!(app.quiet);
    }

    #[test]
    fn manual_refresh_is_not_quiet() {
        let mut app = loaded(snapshot(&["a"], &[], &[]));
        assert_eq!(app.key(Key::Char('r')), Action::Refresh);
        assert!(app.begin_refresh());
        assert!(!app.quiet);
    }

    #[test]
    fn auto_sync_brings_in_messages_and_acks() {
        let mut app = in_conversation(&["a"], &[("m1", "a")]);
        typed(&mut app, "hi");
        let mut s = snapshot(
            &["a"],
            &[("m1", "a"), ("m2", "a")],
            &[sent("s1", "a", true)],
        );
        s.sync = Ok(SyncSummary {
            received: 1,
            acknowledged: 1,
            rejected: 0,
        });
        assert!(app.begin_auto_sync(Instant::now()));
        app.finish(Ok(s));
        assert_eq!(ids(&app.thread("a")), ["m1", "m2", "s1"]);
        assert!(!app.thread("a")[2].awaiting_ack());
        assert_eq!(draft(&app), "hi", "sync leaves the draft alone");
        assert!(app.status.contains("1 new"));
    }

    #[test]
    fn quiet_sync_keeps_a_failure_readable() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "x");
        enter(&mut app);
        app.finish_send(Err("boom".into()));
        assert!(app.begin_auto_sync(Instant::now()));
        app.finish(Ok(snapshot(&["a"], &[], &[])));
        assert!(app.status.contains("boom"));
    }

    #[test]
    fn relay_down_auto_sync_keeps_local_state_and_draft() {
        let mut app = in_conversation(&["a"], &[("m1", "a")]);
        typed(&mut app, "still here");
        let mut s = snapshot(&["a"], &[("m1", "a")], &[sent("s1", "a", true)]);
        s.sync = Err("connection refused".into());
        assert!(app.begin_auto_sync(Instant::now()));
        app.finish(Ok(s));
        assert_eq!(app.relay_ok, Some(false));
        assert!(
            app.status.contains("relay unreachable"),
            "a relay change is news"
        );
        assert_eq!(app.thread("a").len(), 2);
        assert_eq!(draft(&app), "still here");
        assert_eq!(app.focus, Focus::Conversation);

        // A shell that cannot even open keeps the previous snapshot.
        assert!(app.begin_auto_sync(Instant::now()));
        app.finish(Err("locked".into()));
        assert_eq!(app.thread("a").len(), 2);
        assert_eq!(draft(&app), "still here");
    }

    #[test]
    fn enter_goes_down_a_level_and_esc_comes_back() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Enter);
        assert_eq!(app.focus, Focus::Vault);
        assert_eq!(app.selected_message().unwrap().id(), "m1");
        app.key(Key::Esc);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Esc);
        assert_eq!(app.focus, Focus::Contacts);
    }

    #[test]
    fn vault_keys_stay_commands() {
        let mut app = in_conversation(&["a"], &[("m1", "a")]);
        app.key(Key::Enter);
        app.key(Key::Char('j'));
        assert_eq!(app.section(), Section::Trust);
        assert_eq!(app.key(Key::Char('i')), Action::None);
        assert!(!app.composing(), "the vault is for reading");
        assert_eq!(app.key(Key::Char('r')), Action::Refresh);
        assert_eq!(app.key(Key::Backspace), Action::None);
        assert_eq!(app.key(Key::Char('q')), Action::Quit);
    }

    #[test]
    fn newline_breaks_the_draft_and_enter_sends_it_whole() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "later abilities:");
        for item in ["presence", "artifacts"] {
            assert_eq!(app.key(Key::Newline), Action::None, "newline never sends");
            typed(&mut app, item);
        }
        assert_eq!(draft(&app), "later abilities:\npresence\nartifacts");
        app.key(Key::Backspace);
        assert_eq!(draft(&app), "later abilities:\npresence\nartifact");
        let body = enter(&mut app).body;
        assert_eq!(body, "later abilities:\npresence\nartifact");
    }

    #[test]
    fn newlines_alone_are_blank() {
        let mut app = in_conversation(&["a"], &[]);
        app.key(Key::Char('i'));
        app.key(Key::Backspace);
        app.key(Key::Newline);
        app.key(Key::Newline);
        assert_eq!(app.key(Key::Enter), Action::None);
        assert_eq!(draft(&app), "\n\n");
    }

    #[test]
    fn newline_outside_compose_does_nothing() {
        let mut app = loaded(snapshot(&["a"], &[("m1", "a")], &[]));
        assert_eq!(app.key(Key::Newline), Action::None);
        assert!(!app.composing());
        assert_eq!(app.focus, Focus::Contacts);
    }

    #[test]
    fn multiline_draft_survives_failure_and_clears_on_success() {
        let mut app = in_conversation(&["a"], &[]);
        typed(&mut app, "one");
        app.key(Key::Newline);
        typed(&mut app, "two");
        enter(&mut app);
        app.finish_send(Err("down".into()));
        assert_eq!(draft(&app), "one\ntwo");
        enter(&mut app);
        app.finish_send(Ok("s1".into()));
        assert_eq!(draft(&app), "");
        assert_eq!(
            crate::composer::layout(draft(&app), 40).height(crate::composer::MAX_ROWS),
            1
        );
        typed(&mut app, "next");
        assert_eq!(enter(&mut app).body, "next");
    }
}
