//! Rendering. Reads [`App`] and draws it; never touches the network.
//!
//! Layout, left to right like a file manager: contacts, the conversation
//! with the selected contact, and the vault — what the selected message is
//! made of. Regions are separated by hairlines and spacing, not boxes.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::Block;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{
    Activity, App, ComposerView, ContactsView, ConversationView, Drawn, Focus, Label, MessageRows,
    Row, Section, TextRow,
};
use crate::avatar::Glyphs;
use crate::composer::{self, MAX_ROWS};
use crate::snapshot::Peer;
use crate::theme::{NIGHT_GARDEN, Theme};

/// The product's name as shown. The binary and CLI stay `deaddrop`.
pub const WORDMARK: &str = "0xd34ddr0p";

/// Every color on screen comes from here.
const T: Theme = NIGHT_GARDEN;

/// Narrower than this, the vault replaces the contacts column when focused.
const WIDE: u16 = 100;

fn muted() -> Style {
    T.muted()
}

/// Display width of a string, as the terminal will lay it out.
pub(crate) fn width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Shorten text to `max` display columns, marking the cut.
pub fn short(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_owned();
    }
    let mut cut = String::new();
    for c in text.chars() {
        if width(&cut) + width(c.encode_utf8(&mut [0; 4])) + 1 > max {
            break;
        }
        cut.push(c);
    }
    cut.push('…');
    cut
}

/// The human part of a node id (`danil` of `danil:tui:deaddrop`), and the
/// rest. Display only: the whole id is the identity.
pub fn name(id: &str) -> (&str, &str) {
    match id.find(':') {
        Some(i) if i > 0 => id.split_at(i),
        _ => (id, ""),
    }
}

/// Word-wrap to `max` columns; words longer than a line are split.
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    wrap_ranges(text, max)
        .into_iter()
        .map(|r| chars[r].iter().collect())
        .collect()
}

/// [`wrap`] as char ranges into `text`, so a drawn row can be traced back
/// to the exact characters it shows. A space a line broke at belongs to no
/// row; newlines belong to none either.
pub fn wrap_ranges(text: &str, max: usize) -> Vec<std::ops::Range<usize>> {
    let max = max.max(1);
    let chars: Vec<char> = text.chars().collect();
    let cols = |c: char| width(c.encode_utf8(&mut [0; 4]));
    let mut lines = Vec::new();
    let mut p_start = 0;
    for paragraph in text.split('\n') {
        let p_len = paragraph.chars().count();
        // The line being built: chars[start..end], `used` columns wide.
        let (mut start, mut end, mut used) = (p_start, p_start, 0);
        let mut w_start = p_start;
        for word in paragraph.split(' ') {
            let w_len = word.chars().count();
            let w_end = w_start + w_len;
            let w_cols: usize = chars[w_start..w_end].iter().map(|c| cols(*c)).sum();
            let empty = end == start;
            let candidate = if empty { w_cols } else { used + 1 + w_cols };
            if candidate <= max {
                if empty {
                    start = w_start;
                }
                end = w_end;
                used = candidate;
            } else {
                if !empty {
                    lines.push(start..end);
                }
                start = w_start;
                end = w_start;
                used = 0;
                for (i, ch) in chars.iter().enumerate().take(w_end).skip(w_start) {
                    let c = cols(*ch);
                    if used + c > max {
                        lines.push(start..end);
                        start = i;
                        used = 0;
                    }
                    end = i + 1;
                    used += c;
                }
            }
            w_start = w_end + 1;
        }
        lines.push(start..end);
        p_start += p_len + 1;
    }
    lines
}

/// Left and right content on one line, padded apart.
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, cols: usize) -> Line<'static> {
    let used: usize = left.iter().chain(&right).map(Span::width).sum();
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(cols.saturating_sub(used))));
    spans.extend(right);
    Line::from(spans)
}

/// Columns before the draft: the prompt, or its blank continuation.
const PROMPT_COLS: u16 = 3;

/// Columns the draft wraps to on a screen `cols` wide.
fn compose_cols(cols: u16) -> usize {
    cols.saturating_sub(PROMPT_COLS + 1) as usize
}

/// Rows the compose box takes on a `cols` × `rows` screen: one, growing
/// with the wrapped draft up to [`MAX_ROWS`], and never squeezing the
/// conversation below three rows.
pub fn compose_rows(app: &App, cols: u16, rows: u16) -> u16 {
    let Some(compose) = &app.compose else {
        return 1;
    };
    // Header, two rules, footer, and the conversation's minimum.
    let room = (rows as usize).saturating_sub(4 + 3).clamp(1, MAX_ROWS);
    let cursor = composer::cursor_chars(&compose.draft, compose.cursor);
    composer::layout_at(&compose.draft, compose_cols(cols), cursor).height(room) as u16
}

/// Draw one frame. Returns what was drawn where, so scrolling works in the
/// rows the user actually sees and clicks land on what they point at.
pub fn draw(frame: &mut Frame, app: &App, glyphs: Glyphs, tick: usize) -> Drawn {
    let area = frame.area();
    // Deaddrop's own background on every cell, whatever the terminal's theme.
    frame.render_widget(Block::new().style(T.base()), area);
    let [header, rule_top, body, rule_bottom, compose, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(compose_rows(app, area.width, area.height)),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(frame, header, app, glyphs, tick);
    draw_rule(frame, rule_top, glyphs);
    let mut drawn = draw_body(frame, body, app, glyphs, tick);
    match &app.compose {
        // Who it goes to sits on the rule, leaving the box to the draft.
        Some(c) => draw_labelled_rule(
            frame,
            rule_bottom,
            &format!(" to {} ", name(&c.to).0),
            glyphs,
        ),
        None => draw_rule(frame, rule_bottom, glyphs),
    }
    drawn.composer = draw_compose(frame, compose, app, glyphs);
    draw_footer(frame, footer, app, glyphs);
    // Wide glyphs reset the cell they cover. Give every cell without a
    // background of its own (a selection has one) the app's, so no cell ever
    // falls back to the terminal's.
    for cell in frame.buffer_mut().content.iter_mut() {
        if cell.bg == Color::Reset {
            cell.set_bg(T.background);
        }
    }
    drawn
}

fn draw_rule(frame: &mut Frame, area: Rect, glyphs: Glyphs) {
    let line = glyphs.rule().repeat(area.width as usize);
    frame.render_widget(Paragraph::new(line).style(T.divider()), area);
}

fn draw_labelled_rule(frame: &mut Frame, area: Rect, label: &str, glyphs: Glyphs) {
    let cols = area.width as usize;
    let tail = 2.min(cols.saturating_sub(width(label)));
    let head = cols.saturating_sub(width(label) + tail);
    let line = Line::from(vec![
        Span::styled(glyphs.rule().repeat(head), T.divider()),
        Span::styled(label.to_owned(), muted()),
        Span::styled(glyphs.rule().repeat(tail), T.divider()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs, tick: usize) {
    let mut left = vec![
        Span::styled(format!(" {} ", glyphs.mark()), muted()),
        Span::styled(WORDMARK, T.strong(T.you)),
        Span::raw("   "),
    ];
    if let Some(s) = &app.snapshot {
        let (who, rest) = name(&s.node.id);
        left.extend([
            Span::raw(format!("{} ", glyphs.avatar(&s.node.id))),
            Span::raw(who.to_owned()).bold(),
            Span::styled(rest.to_owned(), muted()),
            Span::raw("   "),
        ]);
    }
    left.extend(match app.relay_ok {
        Some(true) => vec![
            Span::styled(glyphs.relay_up(), T.ack()),
            Span::styled(" relay", muted()),
        ],
        Some(false) => vec![
            Span::styled(glyphs.relay_down(), T.error()),
            Span::styled(" relay down", T.error()),
        ],
        None => vec![Span::styled(format!("{} relay", glyphs.idle()), muted())],
    });
    let mut counts = vec![format!("{} peers", app.peers().len())];
    if app.unread_total() > 0 {
        counts.push(format!("{} new", app.unread_total()));
    }
    if app.awaiting_total() > 0 {
        counts.push(format!("{} awaiting ack", app.awaiting_total()));
    }
    left.push(Span::styled(format!("   {}", counts.join(" · ")), muted()));

    // Background syncs stay out of sight; only what the user asked for spins.
    let label = match app.activity {
        _ if app.sending() => Some("sending"),
        Activity::Busy(label) if !app.quiet => Some(label),
        _ => None,
    };
    let right = match label {
        Some(label) => vec![
            Span::styled(glyphs.spinner(tick), Style::new().fg(T.you)),
            Span::styled(format!(" {label} "), muted()),
        ],
        None => vec![],
    };
    frame.render_widget(
        Paragraph::new(spread(left, right, area.width as usize)),
        area,
    );
}

fn draw_body(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs, tick: usize) -> Drawn {
    let wide = area.width >= WIDE;
    let show_contacts = wide || app.focus != Focus::Vault;
    let show_vault = wide || app.focus == Focus::Vault;

    let mut constraints = Vec::new();
    if show_contacts {
        constraints.push(Constraint::Length(if wide { 26 } else { 24 }));
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(20));
    if show_vault {
        constraints.push(Constraint::Length(1));
        constraints.push(Constraint::Length(if wide { 38 } else { 34 }));
    }
    let areas = Layout::horizontal(constraints).split(area);
    let mut next = areas.iter().copied();

    let mut contacts = None;
    if show_contacts {
        contacts = draw_contacts(frame, next.next().unwrap(), app, glyphs);
        draw_separator(frame, next.next().unwrap(), glyphs);
    }
    let conversation = draw_conversation(frame, next.next().unwrap(), app, glyphs, tick);
    if show_vault {
        draw_separator(frame, next.next().unwrap(), glyphs);
        draw_vault(frame, next.next().unwrap(), app, glyphs);
    }
    Drawn {
        conversation: conversation.0,
        contacts,
        composer: None,
        wire_toggle: conversation.1,
    }
}

fn draw_separator(frame: &mut Frame, area: Rect, glyphs: Glyphs) {
    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(glyphs.divider()))
        .collect();
    frame.render_widget(Paragraph::new(lines).style(T.divider()), area);
}

/// Inner area with one column of breathing room on each side.
fn padded(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    }
}

/// A pane heading: `color` when the pane has focus, muted when not.
fn heading(text: &str, focused: bool, color: Color) -> Span<'static> {
    if focused {
        Span::styled(text.to_owned(), T.strong(color))
    } else {
        Span::styled(text.to_owned(), muted())
    }
}

/// A few centered lines in the middle of an area.
fn draw_empty(frame: &mut Frame, area: Rect, lines: Vec<Line<'static>>) {
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let area = Rect {
        y: area.y + top,
        height: area.height - top,
        ..area
    };
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn draw_contacts(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) -> Option<ContactsView> {
    let area = padded(area);
    let focused = app.focus == Focus::Contacts;
    let cols = area.width as usize;
    // With rooms: "rooms", the rooms, a gap, then "peers". Without: as before.
    let room_open = app.open_room().map(|r| r.name.clone());
    let mut lines = Vec::new();
    let mut rooms_at = 0;
    if !app.rooms().is_empty() {
        lines.push(Line::from(heading("rooms", focused, T.text)));
        rooms_at = lines.len();
        for room in app.rooms() {
            let open = room_open.as_deref() == Some(room.name.as_str());
            let gutter = match (open, focused) {
                (true, true) => Span::styled(glyphs.bar(), Style::new().fg(T.you)),
                (true, false) => Span::styled(glyphs.bar(), muted()),
                _ => Span::raw(" "),
            };
            let label = Span::raw(format!("  # {}", room.name));
            lines.push(Line::from(vec![
                gutter,
                if open {
                    label.bold()
                } else {
                    label.style(muted())
                },
            ]));
        }
        lines.push(Line::default());
        lines.push(Line::from(heading("peers", focused, T.text)));
    } else {
        lines.push(Line::from(heading("contacts", focused, T.text)));
        lines.push(Line::default());
    }
    let peers_at = lines.len();

    if app.snapshot.is_some() && app.peers().is_empty() {
        lines.push(Line::styled("no one yet", muted()));
    }
    let names: Vec<&str> = app.peers().iter().map(|p| name(&p.id).0).collect();
    for (i, peer) in app.peers().iter().enumerate() {
        let selected = i == app.contact && room_open.is_none();
        let (who, _) = name(&peer.id);
        // Two peers sharing a first segment show their whole ids.
        let label = if names.iter().filter(|n| **n == who).count() > 1 {
            peer.id.as_str()
        } else {
            who
        };
        let gutter = match (selected, focused) {
            (true, true) => Span::styled(glyphs.bar(), Style::new().fg(T.author(&peer.id))),
            (true, false) => Span::styled(glyphs.bar(), muted()),
            _ => Span::raw(" "),
        };
        let mut right = Vec::new();
        let unread = app.unread_with(&peer.id);
        if unread > 0 {
            right.push(Span::styled(
                format!("{}{unread}", glyphs.unread()),
                Style::new().fg(T.peer),
            ));
        }
        let awaiting = app.awaiting_with(&peer.id);
        if awaiting > 0 {
            right.push(Span::styled(
                format!(" {}{awaiting}", glyphs.awaiting()),
                muted(),
            ));
        }
        let badges: usize = right.iter().map(Span::width).sum();
        let name_cols = cols.saturating_sub(5 + badges);
        let name = Span::raw(short(label, name_cols));
        let name = match (selected, unread > 0) {
            (true, _) => name.bold(),
            (false, true) => name,
            // Read and not picked: a step back.
            (false, false) => name.style(muted()),
        };
        lines.push(spread(
            vec![
                gutter,
                Span::raw(format!(" {} ", glyphs.avatar(&peer.id))),
                name,
            ],
            right,
            cols,
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
    let first = area.y.saturating_add(peers_at as u16);
    let visible = area.bottom().saturating_sub(first);
    let count = u16::try_from(app.peers().len()).unwrap_or(u16::MAX);
    let rooms_y = area.y.saturating_add(rooms_at as u16);
    let room_rows = u16::try_from(app.rooms().len())
        .unwrap_or(u16::MAX)
        .min(area.bottom().saturating_sub(rooms_y));
    (count > 0 || room_rows > 0).then(|| ContactsView {
        x: area.x,
        y: first,
        width: area.width,
        rows: count.min(visible),
        rooms_y,
        room_rows,
    })
}

/// The open conversation (a peer or a room). Returns it as drawn, and the
/// room's `λ wire` label if a room is open.
fn draw_conversation(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    glyphs: Glyphs,
    tick: usize,
) -> (Option<ConversationView>, Option<Label>) {
    let area = padded(area);
    let focused = app.focus == Focus::Conversation;
    let cols = area.width as usize;
    let [head, _, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    let Some(peer) = app.selected_peer() else {
        let lines = match app.snapshot {
            None => vec![Line::styled("waking up…", muted())],
            Some(_) => vec![
                Line::from(glyphs.avatar("")),
                Line::default(),
                Line::from("it's quiet here"),
                Line::styled("trust someone with  deaddrop peer add", muted()),
            ],
        };
        frame.render_widget(
            Paragraph::new(heading("conversation", focused, T.text)),
            head,
        );
        draw_empty(frame, body, lines);
        return (None, None);
    };
    let room = app.open_room().cloned();
    let channel = app.channel().unwrap_or_else(|| peer.id.clone());

    // While scrolled away: how much arrived below, quietly.
    let viewport = app.viewport(&channel);
    let indicator = match (&viewport.anchor, viewport.unseen) {
        (Some(_), 0) => Some(Span::styled("scrolled", muted())),
        (Some(_), n) => Some(Span::styled(format!("+{n} new"), Style::new().fg(T.peer))),
        (None, _) => None,
    };
    let mut toggle = None;
    let who = match &room {
        Some(room) => {
            let label = if app.wire_open {
                "[λ wire]"
            } else {
                " λ wire "
            };
            let mut right: Vec<Span> = indicator.into_iter().collect();
            right.push(Span::raw("  "));
            let wire_style = if app.wire_open {
                Style::new().fg(T.vault).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(T.vault)
            };
            right.push(Span::styled(label, wire_style));
            let w = width(label) as u16;
            toggle = Some(Label {
                x: head.x + head.width.saturating_sub(w),
                y: head.y,
                width: w,
            });
            frame.render_widget(
                Paragraph::new(spread(
                    vec![
                        Span::styled(format!("# {}", room.name), T.strong(T.you)),
                        Span::styled(format!("  {} members", room.members.len()), muted()),
                    ],
                    right,
                    cols,
                )),
                head,
            );
            format!("#{}", room.name)
        }
        None => {
            let (who, rest) = name(&peer.id);
            frame.render_widget(
                Paragraph::new(spread(
                    vec![
                        Span::raw(format!("{} ", glyphs.avatar(&peer.id))),
                        // Their name, so their color — not the focus accent.
                        heading(who, focused, T.author(&peer.id)),
                        Span::styled(rest.to_owned(), muted()),
                    ],
                    vec![indicator.unwrap_or_else(|| Span::styled("via relay", muted()))],
                    cols,
                )),
                head,
            );
            who.to_owned()
        }
    };

    // The room's machine stream: real routing only, read-only.
    if let (Some(room), true) = (&room, app.wire_open) {
        return (draw_wire(frame, body, app, room), toggle);
    }

    let thread = app.current_thread();
    // Your tasks here: the planner while it works, and local notes.
    let task_lines = room
        .as_ref()
        .map(|r| task_lines(app, &r.name, glyphs, tick))
        .unwrap_or_default();
    if thread.is_empty() && task_lines.is_empty() {
        draw_empty(
            frame,
            body,
            vec![
                Line::from(format!("nothing with {who} yet")),
                Line::styled(
                    if room.is_some() {
                        "@mention a member to ask them"
                    } else {
                        "say hello with  deaddrop send"
                    },
                    muted(),
                ),
            ],
        );
        return (None, toggle);
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut blocks: Vec<MessageRows> = Vec::new();
    let mut text_at: Vec<(usize, String, std::ops::Range<usize>)> = Vec::new();
    // Asked members still to report, under the request they answer.
    let pending = if room.is_some() {
        app.pending()
    } else {
        Vec::new()
    };
    // Who wrote each run of messages: you, or that message's real sender.
    let mut last_author: Option<(bool, String)> = None;
    let mut last_era: Option<bool> = None;
    for (i, row) in thread.iter().enumerate() {
        // A message's block starts at the rule or author line above it.
        let start = lines.len();
        let earlier = app.is_earlier(row.id());
        if last_era != Some(earlier) {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            let label = if earlier {
                " earlier · order unknown "
            } else {
                " this session "
            };
            lines.push(centered_rule(label, cols, glyphs));
            last_author = None;
            last_era = Some(earlier);
        }
        // The wire's own messages are not yours, though this node signed them.
        let wire = app.wire_authored(row);
        let author = if wire {
            (true, "λ wire".to_owned())
        } else {
            (row.outgoing(), row.peer().to_owned())
        };
        if last_author.as_ref() == Some(&author) && handoff_target(row).is_some() {
            // A handoff stands apart from the same author's prose above it.
            lines.push(Line::default());
        }
        if last_author.as_ref() != Some(&author) {
            lines.push(Line::default());
            lines.push(if wire {
                Line::from(Span::styled("  λ wire", T.strong(T.vault)))
            } else if row.outgoing() {
                Line::from(Span::styled("  you", T.strong(T.you)))
            } else {
                Line::from(vec![
                    Span::raw(format!("  {} ", glyphs.avatar(row.peer()))),
                    Span::styled(
                        name(row.peer()).0.to_owned(),
                        T.strong(T.author(row.peer())),
                    ),
                ])
            });
            last_author = Some(author);
        }
        for (line, range) in message_lines(app, row, i == app.message, focused, cols, glyphs) {
            if let Some(range) = range {
                text_at.push((lines.len(), row.id().to_owned(), range));
            }
            lines.push(line);
        }
        if row.outgoing()
            && let Some(asked) = row.room()
        {
            for p in pending.iter().filter(|p| p.request == asked.id) {
                lines.push(pending_line(p, glyphs, tick));
            }
        }
        blocks.push(MessageRows {
            id: row.id().to_owned(),
            start,
            end: lines.len(),
        });
    }

    if !task_lines.is_empty() {
        let start = lines.len();
        lines.extend(task_lines);
        blocks.push(MessageRows {
            id: "#task-notes".into(),
            start,
            end: lines.len(),
        });
    }

    let height = body.height as usize;
    // The open channel's own viewport: a room scrolls like any DM.
    let offset = app.scroll_offset(&channel, height, &blocks);
    // Message text starts after the gutter and indent.
    let mut text_rows: Vec<Option<TextRow>> = vec![None; lines.len()];
    for (at, message, range) in text_at {
        text_rows[at] = Some(TextRow {
            message,
            start: range.start,
            end: range.end,
            x: body.x + 4,
        });
    }
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), body);
    let view = Some(ConversationView {
        peer: channel,
        rows: height,
        blocks,
        offset,
        text_rows,
        x: body.x,
        y: body.y,
        width: body.width,
        height: body.height,
    });
    (view, toggle)
}

/// 0xd34ddr0p::wire: the room's real routing, one line per logical room
/// message, newest at the bottom. Read-only; nothing here is inferred.
fn draw_wire(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    room: &deaddrop_room::RoomConfig,
) -> Option<ConversationView> {
    use deaddrop_room::{Kind, Status, short};
    let id8 = |id: &str| id.chars().take(8).collect::<String>();
    let mut lines: Vec<Line<'static>> = vec![
        Line::styled(
            "0xd34ddr0p::wire",
            Style::new().fg(T.vault).add_modifier(Modifier::BOLD),
        ),
        Line::default(),
    ];
    // One line per event; each is a block, so the wire scrolls with the
    // same engine as any conversation, under its own key.
    let mut blocks: Vec<MessageRows> = Vec::new();
    for row in app.room_thread(&room.name) {
        let Some(m) = row.room() else { continue };
        let start = if blocks.is_empty() { 0 } else { lines.len() };
        let from = if app.wire_authored(&row) {
            "λ wire".to_owned()
        } else if row.outgoing() {
            "you".to_owned()
        } else {
            short(row.peer()).to_owned()
        };
        let from_color = author_color(app, &row);
        let (event, to, detail) = match (m.kind, m.status) {
            (Kind::Request, _) => {
                let to: Vec<&str> = m.mentions.iter().map(|x| short(x)).collect();
                let detail = if row.outgoing() {
                    let (acked, total) = app.room_receipts(&m.id);
                    format!("receipts {acked}/{total}")
                } else {
                    // The capability a structured request names, if it names one.
                    let capability = m
                        .text
                        .lines()
                        .next()
                        .and_then(|l| l.strip_prefix("request:: "))
                        .map(|c| format!(" · {c}"))
                        .unwrap_or_default();
                    match &m.reply_to {
                        // An agent's ask, made for an earlier request.
                        Some(r) => format!("hop {}{capability} · for {}", m.hop, id8(r)),
                        None => format!("hop {}{capability}", m.hop),
                    }
                };
                ("agent_request", to.join(","), detail)
            }
            (Kind::Message, _) => ("room_message", format!("#{}", room.name), String::new()),
            (Kind::Report, Some(Status::Refused)) => (
                "request_refused",
                format!("#{}", room.name),
                format!(
                    "re {} · {}",
                    id8(m.reply_to.as_deref().unwrap_or("")),
                    short_text(&m.text)
                ),
            ),
            (Kind::Report, Some(Status::Failed)) => (
                "request_failed",
                format!("#{}", room.name),
                format!("re {}", id8(m.reply_to.as_deref().unwrap_or(""))),
            ),
            (Kind::Report, _) => (
                "agent_report",
                format!("#{}", room.name),
                format!("re {} · ok", id8(m.reply_to.as_deref().unwrap_or(""))),
            ),
        };
        let event_style = match event {
            "request_refused" | "request_failed" => T.error(),
            "agent_request" => Style::new().fg(T.you),
            "agent_report" => Style::new().fg(T.peer),
            _ => muted(),
        };
        // "from → to", then the event and its detail, wrapped to the pane:
        // every drawn row is counted, so long events scroll correctly.
        let cols = area.width as usize;
        for (n, part) in wrap(&format!("{from} → {to}"), cols)
            .into_iter()
            .enumerate()
        {
            lines.push(if n == 0 && part.starts_with(&from) {
                Line::from(vec![
                    Span::styled(from.clone(), Style::new().fg(from_color)),
                    Span::raw(part[from.len()..].to_owned()),
                ])
            } else if n == 0 {
                Line::raw(part)
            } else {
                Line::styled(part, muted())
            });
        }
        let detail = format!("{event}  {}  {detail}", id8(&m.id));
        for (n, part) in wrap(&detail, cols.saturating_sub(2))
            .into_iter()
            .enumerate()
        {
            lines.push(if n == 0 && part.starts_with(event) {
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(event.to_owned(), event_style),
                    Span::styled(part[event.len()..].to_owned(), muted()),
                ])
            } else {
                Line::styled(format!("  {part}"), muted())
            });
        }
        blocks.push(MessageRows {
            id: row.id().to_owned(),
            start,
            end: lines.len(),
        });
    }
    // Each task's orchestration, event by event: only what really happened.
    for (id, events) in app.task_traces(&room.name) {
        let start = if blocks.is_empty() { 0 } else { lines.len() };
        lines.push(Line::default());
        lines.push(Line::styled(
            format!("task:: {id}"),
            Style::new().fg(T.vault).add_modifier(Modifier::BOLD),
        ));
        blocks.push(MessageRows {
            id: format!("{id}#head"),
            start,
            end: lines.len(),
        });
        let color = |who: &str| {
            room.members
                .iter()
                .find(|m| short(m) == who)
                .map_or(T.vault, |m| T.author(m))
        };
        for (n, e) in events.iter().enumerate() {
            let start = lines.len();
            let mut head = vec![Span::styled(
                e.from.clone(),
                Style::new().fg(color(&e.from)),
            )];
            if let Some(to) = &e.to {
                head.push(Span::styled(" → ", muted()));
                head.push(Span::styled(to.clone(), Style::new().fg(color(to))));
            }
            lines.push(Line::from(head));
            for (key, value) in &e.fields {
                for (k, part) in wrap(
                    &format!("{key}:: {value}"),
                    area.width.saturating_sub(2) as usize,
                )
                .into_iter()
                .enumerate()
                {
                    let label = format!("{key}:: ");
                    lines.push(if let (0, Some(rest)) = (k, part.strip_prefix(&label)) {
                        Line::from(vec![
                            Span::raw("  "),
                            Span::styled(label.clone(), muted()),
                            Span::raw(rest.to_owned()),
                        ])
                    } else {
                        Line::styled(format!("  {part}"), muted())
                    });
                }
            }
            blocks.push(MessageRows {
                id: format!("{id}#{n}"),
                start,
                end: lines.len(),
            });
        }
    }
    if blocks.is_empty() {
        lines.push(Line::styled("no routing yet", muted()));
        frame.render_widget(Paragraph::new(lines), area);
        return None;
    }
    let key = format!("#{}::wire", room.name);
    let height = area.height as usize;
    let offset = app.scroll_offset(&key, height, &blocks);
    let rows = lines.len();
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), area);
    Some(ConversationView {
        peer: key,
        rows: height,
        blocks,
        offset,
        // Read-only: nothing to select.
        text_rows: vec![None; rows],
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height,
    })
}

/// Your task lines in a room: the planner while it plans (truthful: only
/// that it is planning, and for how long), then local notes such as a dry
/// run's plan or a refusal. Shown to you only; nothing here was sent.
fn task_lines(app: &App, room: &str, glyphs: Glyphs, tick: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Local and the wire's own: never under `you`.
    let header = || {
        Line::from(vec![
            Span::styled("  λ wire", T.strong(T.vault)),
            Span::styled(" · only you see this", muted()),
        ])
    };
    for note in app.task_notes(room) {
        lines.push(Line::default());
        lines.push(header());
        for l in &note.lines {
            let mut spans = vec![Span::raw("    ")];
            spans.extend(handoff_spans(l, None));
            lines.push(Line::from(spans));
        }
    }
    if let Some(p) = app.task_planning(room) {
        lines.push(Line::default());
        lines.push(header());
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(glyphs.typing(tick).to_owned(), Style::new().fg(T.vault)),
            Span::styled(" planner", Style::new().fg(T.vault)),
            Span::styled(
                format!(
                    " · planning {}{} · {}s",
                    p.id,
                    if p.dry_run { " · dry run" } else { "" },
                    p.since.elapsed().as_secs()
                ),
                muted(),
            ),
        ]));
    }
    lines
}

/// `[···] @research · received · awaiting reply · 12s`. The dots mean only
/// that the request is still pending — not thinking, searching or
/// inspecting. `sent` until the agent acknowledges receipt, then `received ·
/// awaiting reply`; the line is gone once its reply, refusal or failure is
/// in. No progress is invented.
fn pending_line(p: &crate::app::Pending, glyphs: Glyphs, tick: usize) -> Line<'static> {
    let state = if p.received {
        "received · awaiting reply"
    } else {
        "sent"
    };
    let elapsed = p
        .since
        .map(|t| format!(" · {}s", t.elapsed().as_secs()))
        .unwrap_or_default();
    Line::from(vec![
        Span::raw("      "),
        Span::styled(
            glyphs.typing(tick).to_owned(),
            Style::new().fg(T.author(&p.member)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("@{}", deaddrop_room::short(&p.member)),
            Style::new().fg(T.author(&p.member)),
        ),
        Span::styled(format!(" · {state}{elapsed}"), muted()),
    ])
}

/// Who wrote `row`, as a color: the wire, you, or that peer's own color.
fn author_color(app: &App, row: &Row<'_>) -> Color {
    if app.wire_authored(row) {
        T.vault
    } else if row.outgoing() {
        T.you
    } else {
        T.author(row.peer())
    }
}

fn short_text(text: &str) -> String {
    short(text.lines().next().unwrap_or(""), 40)
}

fn centered_rule(label: &str, cols: usize, glyphs: Glyphs) -> Line<'static> {
    let side = cols.saturating_sub(width(label)) / 2;
    let rule = glyphs.rule().repeat(side.min(12));
    Line::from(vec![
        Span::styled(rule.clone(), T.divider()),
        Span::styled(label.to_owned(), muted()),
        Span::styled(rule, T.divider()),
    ])
    .alignment(Alignment::Center)
}

/// A message's drawn lines, each with the char range of the body it shows
/// (`None` for the "(no body)" placeholder).
fn message_lines(
    app: &App,
    row: &Row<'_>,
    selected: bool,
    focused: bool,
    cols: usize,
    glyphs: Glyphs,
) -> Vec<(Line<'static>, Option<std::ops::Range<usize>>)> {
    let mut meta: Vec<Span<'static>> = Vec::new();
    if row.kind() != "message" {
        meta.push(Span::styled(format!(" {}", row.kind()), muted()));
    }
    if !row.artifacts().is_empty() {
        meta.push(Span::styled(
            format!(" {}{}", glyphs.artifact(), row.artifacts().len()),
            muted(),
        ));
    }
    let room = row.room();
    if let Some(r) = room
        .as_ref()
        .filter(|r| r.status.is_some_and(|s| s != deaddrop_room::Status::Ok))
    {
        let status = r.status.map_or("", |s| s.as_str());
        meta.push(Span::styled(format!(" {status}"), T.error()));
    }
    if let (Row::Sent(_), Some(r)) = (row, room.as_ref()) {
        // A room message is one delivery per member: receipts, k of n.
        let (acked, total) = app.room_receipts(&r.id);
        meta.push(if acked == 0 {
            Span::styled(format!(" {}0/{total}", glyphs.awaiting()), muted())
        } else {
            Span::styled(format!(" {}{acked}/{total}", glyphs.check()), T.ack())
        });
    } else if let Row::Sent(m) = row {
        meta.push(if m.acked_by.is_empty() {
            Span::styled(format!(" {}", glyphs.awaiting()), muted())
        } else {
            Span::styled(format!(" {}", glyphs.check()), T.ack())
        });
    }
    let meta_cols: usize = meta.iter().map(Span::width).sum();
    // The selection bar takes the author's color, so picking a message
    // never hides which side it came from.
    let gutter = if selected && focused {
        Span::styled(
            format!("{} ", glyphs.bar()),
            Style::new().fg(author_color(app, row)),
        )
    } else if selected {
        Span::styled(format!("{} ", glyphs.bar()), muted())
    } else if app.is_unread(row.id()) {
        Span::styled(format!("{} ", glyphs.unread()), Style::new().fg(T.peer))
    } else {
        Span::raw("  ")
    };

    let text_cols = cols.saturating_sub(4 + meta_cols);
    if let Some(digest) = room.as_ref().and_then(task_digest) {
        let all: Vec<String> = digest.iter().flat_map(|l| wrap(l, text_cols)).collect();
        let last = all.len().saturating_sub(1);
        return all
            .iter()
            .enumerate()
            .map(|(n, l)| {
                let mut left = vec![gutter.clone(), Span::raw("  ")];
                left.extend(handoff_spans(l, None));
                let line = if n == last {
                    spread(left, meta.clone(), cols)
                } else {
                    Line::from(left)
                };
                (line, None)
            })
            .collect();
    }
    let chars: Vec<char> = row.body().chars().collect();
    let ranges = if chars.is_empty() {
        std::iter::once(0..0).collect()
    } else {
        wrap_ranges(row.body(), text_cols)
    };
    let picked = app.selected_chars(row.id());
    // An agent's structured request to another agent: shown as a handoff,
    // field by field, from exactly the text it carries.
    // Structured lines: a handoff (with its target), or a task message.
    let fields: Option<Option<String>> = match handoff_target(row) {
        Some(to) => Some(Some(to)),
        None => room
            .as_ref()
            .filter(|r| r.text.lines().any(|l| l.starts_with("task:: ")))
            .map(|_| None),
    };
    let last = ranges.len() - 1;
    ranges
        .into_iter()
        .enumerate()
        .map(|(n, range)| {
            let mut left = vec![gutter.clone(), Span::raw("  ")];
            if chars.is_empty() {
                left.push(Span::styled(
                    "(no body)",
                    muted().add_modifier(Modifier::ITALIC),
                ));
            } else if let (Some(to), None) = (&fields, picked) {
                left.extend(handoff_spans(
                    &chars[range.clone()].iter().collect::<String>(),
                    to.as_deref(),
                ));
            } else {
                left.extend(highlighted(&chars, range.clone(), picked));
            }
            let line = if n == last {
                spread(left, meta.clone(), cols)
            } else {
                Line::from(left)
            };
            (line, (!chars.is_empty()).then_some(range))
        })
        .collect()
}

/// The peer an agent's structured request (a handoff) is addressed to.
fn handoff_target(row: &Row<'_>) -> Option<String> {
    row.room()
        .filter(|r| !row.outgoing() && r.kind == deaddrop_room::Kind::Request && r.hop >= 1)
        .filter(|r| r.text.starts_with("request:: "))
        .and_then(|r| r.mentions.first().cloned())
}

/// A synthesis step's request carries whole reports to its worker. In the
/// room it shows as what it is: the objective, and which reports it carries.
/// The full text is still the signed message (vault › raw envelope).
fn task_digest(m: &deaddrop_room::RoomMessage) -> Option<Vec<String>> {
    if m.kind != deaddrop_room::Kind::Request
        || !m.text.starts_with("task:: ")
        || !m.text.contains("\nREPORT ")
    {
        return None;
    }
    let mut lines = vec![m.text.lines().next()?.to_owned()];
    if let Some(objective) = m.text.lines().find(|l| l.starts_with("synthesize:: ")) {
        lines.push(objective.to_owned());
    }
    let reports: Vec<&str> = m
        .text
        .lines()
        .filter_map(|l| l.strip_prefix("REPORT "))
        .collect();
    lines.push(format!("reports:: {}", reports.join(", ")));
    Some(lines)
}

/// One line of a handoff: `→ @peer` in that peer's color, `key::` muted.
fn handoff_spans(line: &str, to: Option<&str>) -> Vec<Span<'static>> {
    if let (Some(rest), Some(to)) = (line.strip_prefix("→ "), to) {
        return vec![
            Span::styled("→ ", muted()),
            Span::styled(rest.to_owned(), Style::new().fg(T.author(to))),
        ];
    }
    match line.split_once(":: ") {
        Some((key, value))
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_lowercase() || c == '_') =>
        {
            let value = Span::raw(value.to_owned());
            vec![
                Span::styled(format!("{key}:: "), muted()),
                if matches!(key, "request" | "task") {
                    value.bold()
                } else {
                    value
                },
            ]
        }
        _ => vec![Span::raw(line.to_owned())],
    }
}

/// `chars[range]`, with the selected part (inclusive char bounds) on the
/// selection surface. Only selected text changes; nothing else does.
fn highlighted(
    chars: &[char],
    range: std::ops::Range<usize>,
    picked: Option<(usize, usize)>,
) -> Vec<Span<'static>> {
    let text = |r: std::ops::Range<usize>| chars[r].iter().collect::<String>();
    let Some((a, b)) = picked else {
        return vec![Span::raw(text(range))];
    };
    let lo = a.max(range.start);
    let hi = b.saturating_add(1).min(range.end);
    if lo >= hi {
        return vec![Span::raw(text(range))];
    }
    let picked_style = Style::new().fg(T.text).bg(T.selection_surface);
    [
        Span::raw(text(range.start..lo)),
        Span::styled(text(lo..hi), picked_style),
        Span::raw(text(hi..range.end)),
    ]
    .into_iter()
    .filter(|s| !s.content.is_empty())
    .collect()
}

fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<8}"), muted()),
        Span::raw(value.into()),
    ])
}

/// Long opaque values (keys, refs, encodings) broken across lines.
fn block(text: &str, cols: usize) -> Vec<Line<'static>> {
    let cols = cols.saturating_sub(2).max(1);
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(cols)
        .map(|c| Line::from(format!("  {}", c.iter().collect::<String>())))
        .collect()
}

/// Quiet explanatory prose, wrapped and indented.
fn note(text: &str, cols: usize) -> Vec<Line<'static>> {
    wrap(text, cols.saturating_sub(2))
        .into_iter()
        .map(|l| Line::styled(format!("  {l}"), muted()))
        .collect()
}

fn peer_key<'a>(app: &'a App, id: &str) -> Option<&'a Peer> {
    app.peers().iter().find(|p| p.id == id)
}

fn draw_vault(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let area = padded(area);
    let focused = app.focus == Focus::Vault;
    let cols = area.width as usize;
    let mut lines = Vec::new();

    let message = match app.focus {
        Focus::Contacts => None,
        _ => app.selected_message(),
    };
    match (message, app.selected_peer()) {
        (Some(row), _) => {
            lines.push(Line::from(vec![
                heading("vault", focused, T.vault),
                Span::styled(" · message", muted()),
            ]));
            lines.push(Line::default());
            for (i, section) in Section::ALL.into_iter().enumerate() {
                let open = focused && i == app.section;
                let (marker, label) = if open {
                    (
                        Span::styled(format!("{} ", glyphs.expanded()), Style::new().fg(T.vault)),
                        Span::raw(section.label()).bold(),
                    )
                } else {
                    (
                        Span::styled(format!("{} ", glyphs.collapsed()), muted()),
                        Span::raw(section.label()),
                    )
                };
                let summary = summary(app, row, section, glyphs);
                let room = cols.saturating_sub(3 + section.label().len());
                lines.push(spread(
                    vec![marker, label],
                    vec![Span::styled(short(&summary, room), muted())],
                    cols,
                ));
                if open {
                    lines.extend(expanded(app, row, section, cols));
                    lines.push(Line::default());
                }
            }
        }
        (None, Some(peer)) => {
            lines.push(Line::from(vec![
                heading("vault", focused, T.vault),
                Span::styled(" · contact", muted()),
            ]));
            lines.push(Line::default());
            lines.push(Line::from("identity"));
            lines.extend(block(&peer.id, cols));
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::raw("trust  "),
                Span::styled("explicit · out of band", muted()),
            ]));
            lines.extend(block(&peer.key, cols));
            lines.push(Line::default());
            let thread = app.thread(&peer.id);
            let sent = thread.iter().filter(|r| r.outgoing()).count();
            lines.push(Line::from("thread"));
            lines.push(field("in", (thread.len() - sent).to_string()));
            lines.push(field("out", sent.to_string()));
            lines.push(field("waiting", app.awaiting_with(&peer.id).to_string()));
            lines.push(Line::default());
            lines.extend(note("avatars are decoration, not identity", cols));
        }
        (None, None) => {
            lines.push(Line::from(heading("vault", focused, T.vault)));
            lines.push(Line::default());
            lines.push(Line::styled("nothing selected", muted()));
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn summary(app: &App, row: Row<'_>, section: Section, glyphs: Glyphs) -> String {
    match section {
        Section::Identity => short(row.id(), 12),
        Section::Trust => match peer_key(app, row.peer()) {
            Some(_) => "explicit peer".into(),
            None => "not a trusted peer".into(),
        },
        Section::Signatures => match row {
            Row::Received(_) => "verified".into(),
            Row::Sent(_) => "signed by you".into(),
        },
        Section::Artifacts => match row.artifacts().len() {
            0 => "none".into(),
            n => format!("{} {n}", glyphs.artifact()),
        },
        Section::Delivery => match row {
            Row::Sent(m) if m.acked_by.is_empty() => format!("{} awaiting ack", glyphs.awaiting()),
            Row::Sent(_) => format!("{} acked", glyphs.check()),
            Row::Received(_) => "received".into(),
        },
        Section::Correlation => row.correlation().unwrap_or("none").to_owned(),
        Section::Raw => format!("{} bytes", row.raw().len()),
    }
}

fn expanded(app: &App, row: Row<'_>, section: Section, cols: usize) -> Vec<Line<'static>> {
    match section {
        Section::Identity => {
            let me = app.snapshot.as_ref().map_or("", |s| s.node.id.as_str());
            let (from, to) = if row.outgoing() {
                (me, row.peer())
            } else {
                (row.peer(), me)
            };
            let mut lines = block(row.id(), cols);
            lines.push(field("kind", row.kind()));
            lines.push(field("from", from));
            lines.push(field("to", to));
            lines
        }
        Section::Trust => match peer_key(app, row.peer()) {
            Some(peer) => {
                let mut lines = vec![field("peer", name(&peer.id).0)];
                lines.extend(block(&peer.key, cols));
                lines.extend(note("trusted for authentication only", cols));
                lines
            }
            None => vec![Line::styled("  no trusted key on file", muted())],
        },
        Section::Signatures => {
            let mut lines = vec![Line::from("  ed25519, detached")];
            lines.extend(note(
                match row {
                    Row::Received(_) => "checked by the shell before it was kept",
                    Row::Sent(_) => "made with this node's key at send",
                },
                cols,
            ));
            lines.extend(note("records are not exposed by the shell", cols));
            lines
        }
        Section::Artifacts if row.artifacts().is_empty() => {
            vec![Line::styled("  no artifact refs", muted())]
        }
        Section::Artifacts => row
            .artifacts()
            .iter()
            .flat_map(|a| block(a, cols))
            .collect(),
        Section::Delivery => {
            let mut lines: Vec<Line> = row
                .delivery()
                .iter()
                .map(|kind| Line::from(format!("  {}", kind.replace('_', " "))))
                .collect();
            if let Row::Sent(m) = row {
                if m.acked_by.is_empty() {
                    lines.push(Line::styled("  no ack recorded yet", muted()));
                } else {
                    let by: Vec<&str> = m.acked_by.iter().map(|id| name(id).0).collect();
                    lines.push(field("ack by", by.join(", ")));
                }
                lines.extend(note("ack = receipt, not task success", cols));
            }
            lines
        }
        Section::Correlation => match row.correlation() {
            Some(c) => block(c, cols),
            None => vec![Line::styled("  none", muted())],
        },
        Section::Raw => block(row.raw(), cols)
            .into_iter()
            .map(|l| l.style(muted()))
            .collect(),
    }
}

/// The composer: a clickable input. Returns where it is, for clicks.
fn draw_compose(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) -> Option<ComposerView> {
    // The compose line is yours: same cool accent as your messages.
    let prompt = Span::styled(format!(" {} ", glyphs.prompt()), Style::new().fg(T.you));
    let mut clickable = ComposerView {
        x: area.x,
        y: area.y,
        width: area.width,
        height: area.height,
        text_x: area.x + PROMPT_COLS,
        cols: compose_cols(area.width),
        offset: 0,
    };
    let Some(compose) = &app.compose else {
        let hint = match app.selected_peer() {
            Some(_) => "drop message",
            None => "pick a contact",
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                prompt,
                Span::styled(hint, muted().add_modifier(Modifier::ITALIC)),
            ])),
            area,
        );
        return app.selected_peer().map(|_| clickable);
    };

    let cursor = composer::cursor_chars(&compose.draft, compose.cursor);
    let view = composer::layout_at(&compose.draft, compose_cols(area.width), cursor);
    let height = (area.height as usize).max(1);
    let offset = view.offset(height);
    clickable.offset = offset;
    let lines: Vec<Line> = view
        .rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(n, row)| {
            let lead = if n == offset {
                prompt.clone()
            } else {
                Span::raw(" ".repeat(PROMPT_COLS as usize))
            };
            let text = if compose.sending {
                Span::styled(row.clone(), muted())
            } else {
                Span::raw(row.clone())
            };
            Line::from(vec![lead, text])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    if !compose.sending {
        let (row, col) = view.cursor;
        let x = (area.x + PROMPT_COLS + col as u16).min(area.right().saturating_sub(1));
        let y = area.y + (row - offset) as u16;
        frame.set_cursor_position((x, y));
    }
    Some(clickable)
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let up_down = match glyphs {
        Glyphs::Unicode => "↑↓",
        Glyphs::Ascii => "up/dn",
    };
    let keys: &[(&str, &str)] = match (app.compose.as_ref(), app.focus) {
        // Only keys that work in every terminal; see `input`.
        // The composing dialect. Ctrl+V and Ctrl+Shift+V arrive as a paste
        // in Windows Terminal; a right click does not paste while the app
        // has the mouse (it is reserved for a composer menu).
        (Some(_), _) => &[
            ("enter::", "drop"),
            ("ctrl+j::", "newline"),
            ("ctrl+v::", "paste"),
            ("esc::", "done"),
        ],
        (None, Focus::Contacts) => &[
            (up_down, "contacts"),
            ("enter", "open"),
            ("r", "sync"),
            ("q", "quit"),
        ],
        (None, Focus::Conversation) => &[
            (up_down, "messages"),
            ("wheel", "scroll"),
            ("enter", "detail"),
            ("esc", "contacts"),
            ("type", "to write"),
        ],
        (None, Focus::Vault) => &[
            (up_down, "sections"),
            ("esc", "back"),
            ("r", "sync"),
            ("q", "quit"),
        ],
    };
    let mut left_spans = vec![Span::raw(" ")];
    for (key, what) in keys {
        left_spans.push(Span::styled((*key).to_owned(), T.text().bold()));
        left_spans.push(Span::styled(format!(" {what}   "), muted()));
    }
    let used: usize = left_spans.iter().map(Span::width).sum();
    let room = (area.width as usize).saturating_sub(used + 1);
    let status = Span::styled(
        format!("{} ", short(&app.status, room)),
        T.status(&app.status),
    );
    frame.render_widget(
        Paragraph::new(spread(left_spans, vec![status], area.width as usize)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_keeps_short_text_and_marks_cuts() {
        assert_eq!(short("abc", 5), "abc");
        assert_eq!(short("abcdef", 4), "abc…");
        assert_eq!(short("", 3), "");
    }

    #[test]
    fn name_is_the_first_id_segment() {
        assert_eq!(name("danil:tui:deaddrop"), ("danil", ":tui:deaddrop"));
        assert_eq!(name("plain"), ("plain", ""));
        assert_eq!(name(":odd"), (":odd", ""));
    }

    #[test]
    fn wrap_breaks_on_words_and_splits_long_ones() {
        assert_eq!(wrap("hello there world", 11), ["hello there", "world"]);
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("one\ntwo", 10), ["one", "two"]);
        assert_eq!(wrap("", 10), [""]);
    }

    #[test]
    fn wrap_counts_wide_glyphs_as_two_columns() {
        assert_eq!(wrap("🌼🌼🌼", 4), ["🌼🌼", "🌼"]);
    }

    use crate::app::{Action, Drawn, Key, Outgoing, WHEEL_ROWS};
    use crate::snapshot::{Node, Received, Snapshot, SyncSummary};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn chatting(draft: &str) -> App {
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(Snapshot {
            node: Node {
                id: "iva:local:deaddrop".into(),
                key: "ed25519:iva".into(),
                relay: "http://127.0.0.1:8787".into(),
            },
            peers: vec![Peer {
                id: "danil:local:deaddrop".into(),
                key: "ed25519:danil".into(),
            }],
            inbox: vec![Received {
                id: "m1".into(),
                from: "danil:local:deaddrop".into(),
                kind: "message".into(),
                body: "hello iva".into(),
                correlation: None,
                artifacts: vec![],
                delivery: vec![],
                raw: String::new(),
            }],
            sent: vec![],
            sync: Ok(SyncSummary::default()),
        }));
        app.key(Key::Enter);
        for c in draft.chars() {
            app.key(if c == '\n' {
                Key::Newline
            } else {
                Key::Char(c)
            });
        }
        app
    }

    /// Render and return the screen as plain text rows.
    fn screen(app: &App, cols: u16, rows: u16, glyphs: Glyphs) -> (Vec<String>, (u16, u16)) {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|f| {
                draw(f, app, glyphs, 0);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = (0..rows)
            .map(|y| (0..cols).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        let cursor = terminal.get_cursor_position().unwrap();
        (text, (cursor.x, cursor.y))
    }

    #[test]
    fn compose_box_is_one_row_until_the_draft_wraps() {
        assert_eq!(compose_rows(&chatting(""), 140, 24), 1);
        assert_eq!(compose_rows(&chatting("hello danil"), 140, 24), 1);
        let long = "word ".repeat(60);
        assert_eq!(compose_rows(&chatting(&long), 140, 24), 3);
        assert_eq!(
            compose_rows(&chatting(&"x\n".repeat(20)), 140, 24),
            MAX_ROWS as u16
        );
        assert_eq!(compose_rows(&App::new(), 140, 24), 1, "not composing");
    }

    #[test]
    fn a_short_screen_keeps_room_for_the_conversation() {
        let tall = "x\n".repeat(20);
        assert_eq!(compose_rows(&chatting(&tall), 80, 12), 5);
        assert_eq!(compose_rows(&chatting(&tall), 80, 6), 1);
    }

    #[test]
    fn tall_composer_leaves_panes_and_cursor_in_place() {
        let draft =
            "later abilities for o-dzi:\npresence\ncapabilities\nartifacts\nmemory\nmore\nend";
        let app = chatting(draft);
        let (rows, (x, y)) = screen(&app, 140, 24, Glyphs::Unicode);
        assert!(rows[2].contains("contacts") && rows[2].contains("danil"));
        assert!(
            rows.iter().any(|r| r.contains("hello iva")),
            "conversation still drawn"
        );
        let rule = 24 - 1 - MAX_ROWS - 1;
        assert!(
            rows[rule].contains("to danil"),
            "recipient sits on the rule"
        );
        // Seven rows of draft in six: the first scrolled away, the end shown.
        assert!(rows[rule + 1].contains("presence"));
        assert!(rows[rule + 1].starts_with(" › "));
        assert!(rows[22].contains("end"));
        assert!(rows[23].contains("enter"), "footer kept");
        assert_eq!((x, y), (3 + 3, 22), "cursor right after \"end\"");
    }

    #[test]
    fn cursor_follows_soft_wraps() {
        let app = chatting(&"abc ".repeat(40));
        let (rows, (x, y)) = screen(&app, 60, 20, Glyphs::Unicode);
        let view = composer::layout(&"abc ".repeat(40), compose_cols(60));
        let height = compose_rows(&app, 60, 20) as usize;
        assert_eq!(height, view.rows.len());
        assert_eq!(y as usize, 20 - 1 - height + view.cursor.0);
        assert_eq!(x as usize, 3 + view.cursor.1);
        assert!(rows[19].contains("enter"));
    }

    #[test]
    fn narrow_and_ascii_screens_render() {
        let draft = "🌼 привет wide glyphs and a long enough line to wrap\nnext";
        for (cols, rows) in [(40, 12), (24, 8), (10, 6), (3, 4)] {
            for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
                let app = chatting(draft);
                let (text, _) = screen(&app, cols, rows, glyphs);
                assert_eq!(text.len(), rows as usize);
            }
        }
        let (text, _) = screen(&chatting(draft), 40, 12, Glyphs::Ascii);
        assert!(text.iter().any(|r| r.starts_with(" > ")));
    }

    /// The finished frame, as drawn — including cells a wide glyph covers,
    /// which never reach the backend.
    fn buffer(app: &App, cols: u16, rows: u16, glyphs: Glyphs) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|f| {
                draw(f, app, glyphs, 0);
            })
            .unwrap()
            .buffer
            .clone()
    }

    /// Foreground of the first cell of `needle` on screen.
    fn fg_of(buffer: &ratatui::buffer::Buffer, needle: &str) -> Color {
        fg_at(buffer, needle, 0)
    }

    /// Foreground `skip` characters into the first `needle` on screen.
    fn fg_at(buffer: &ratatui::buffer::Buffer, needle: &str, skip: u16) -> Color {
        let area = buffer.area;
        for y in 0..area.height {
            let row: Vec<&str> = (0..area.width).map(|x| buffer[(x, y)].symbol()).collect();
            let text = row.concat();
            if let Some(byte) = text.find(needle) {
                let x = text[..byte].chars().count() as u16;
                return buffer[(x + skip, y)].fg;
            }
        }
        panic!("{needle:?} not on screen");
    }

    #[test]
    fn every_cell_has_the_night_garden_background() {
        let tall = "a tall draft\n".repeat(10);
        for (app, cols, rows) in [
            (App::new(), 120, 30),
            (chatting(""), 140, 40),
            (chatting(&tall), 60, 20),
        ] {
            for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
                let buffer = buffer(&app, cols, rows, glyphs);
                for (i, cell) in buffer.content().iter().enumerate() {
                    let (x, y) = buffer.pos_of(i);
                    assert_eq!(
                        cell.bg,
                        T.background,
                        "({x},{y}) {cols}x{rows} {:?}",
                        cell.symbol()
                    );
                }
            }
        }
    }

    #[test]
    fn authors_and_bodies_take_their_theme_colors() {
        let mut app = chatting("");
        app.finish_send(Ok("s1".into()));
        let mut snapshot = app.snapshot.clone().unwrap();
        snapshot.sent.push(crate::snapshot::Sent {
            id: "s1".into(),
            to: "danil:local:deaddrop".into(),
            kind: "message".into(),
            body: "hi danil".into(),
            correlation: None,
            artifacts: vec![],
            delivery: vec![],
            acked_by: vec![],
            raw: String::new(),
        });
        app.begin_refresh();
        app.finish(Ok(snapshot));
        let buffer = buffer(&app, 140, 30, Glyphs::Unicode);
        assert_eq!(fg_at(&buffer, "│   you", 4), T.you);
        // The peer's name heads their message, one row above its body.
        let row = |y: u16| -> String {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        };
        let body_y = (0..buffer.area.height)
            .find(|y| row(*y).contains("hello iva"))
            .unwrap();
        let author = row(body_y - 1);
        let x = author[..author.rfind("danil").unwrap()].chars().count() as u16;
        assert_eq!(
            buffer[(x, body_y - 1)].fg,
            T.author(DANIL),
            "their identity color"
        );
        assert_eq!(fg_of(&buffer, "hello iva"), T.text, "bodies stay plain");
        assert_eq!(fg_of(&buffer, "hi danil"), T.text);
        assert_eq!(fg_of(&buffer, WORDMARK), T.you);
        assert_eq!(fg_of(&buffer, "◷"), T.muted, "awaiting ACK is quiet");
        assert_eq!(fg_of(&buffer, "›"), T.you, "compose is yours");
    }

    #[test]
    fn relay_down_is_the_only_red_in_the_header() {
        let mut app = chatting("");
        let mut snapshot = app.snapshot.clone().unwrap();
        snapshot.sync = Err("connection refused".into());
        app.begin_refresh();
        app.finish(Ok(snapshot));
        let buffer = buffer(&app, 140, 30, Glyphs::Unicode);
        assert_eq!(fg_of(&buffer, "relay down"), T.error);
        assert_eq!(fg_of(&buffer, "relay unreachable"), T.error);
    }

    #[test]
    fn header_shows_the_wordmark_in_both_glyph_sets() {
        let app = chatting("");
        let (rows, _) = screen(&app, 140, 24, Glyphs::Unicode);
        assert!(rows[0].starts_with(" [↓] 0xd34ddr0p"), "{:?}", rows[0]);
        let (rows, _) = screen(&app, 140, 24, Glyphs::Ascii);
        assert!(rows[0].starts_with(" [v] 0xd34ddr0p"), "{:?}", rows[0]);
        assert!(rows[0].is_ascii());
        assert!(!rows[0].contains(" deaddrop "), "old wordmark gone");
    }

    #[test]
    fn footer_offers_only_the_newline_key_that_always_works() {
        let (rows, _) = screen(&chatting("hi"), 140, 24, Glyphs::Unicode);
        let footer = &rows[23];
        for hint in [
            "enter:: drop",
            "ctrl+j:: newline",
            "ctrl+v:: paste",
            "esc:: done",
        ] {
            assert!(footer.contains(hint), "{hint:?} in {footer:?}");
        }
        assert!(!footer.to_lowercase().contains("shift"));
    }

    #[test]
    fn ctrl_j_events_build_a_multiline_draft_that_wraps_and_scrolls() {
        use crate::input::{Input, translate};
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        fn feed(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> crate::app::Action {
            let Some(Input::Key(key)) = translate(KeyEvent::new(code, modifiers)) else {
                panic!("not a key");
            };
            app.key(key)
        }
        let mut app = chatting("");
        let lines = [
            "later abilities for o-dzi:",
            "presence",
            "capabilities",
            "artifacts",
            "conversation memory and a tail long enough to wrap in sixty columns",
            "last",
        ];
        for (n, line) in lines.iter().enumerate() {
            if n > 0 {
                assert_eq!(
                    feed(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL),
                    crate::app::Action::None,
                    "ctrl+j never sends"
                );
            }
            for c in line.chars() {
                feed(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
            }
        }
        assert_eq!(app.compose.as_ref().unwrap().draft, lines.join("\n"));
        assert_eq!(compose_rows(&app, 60, 24), MAX_ROWS as u16);
        let (rows, (x, y)) = screen(&app, 60, 24, Glyphs::Unicode);
        assert!(rows[22].contains("last"));
        assert_eq!((x, y), (3 + 4, 22), "cursor after \"last\"");

        let send = feed(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(matches!(send, crate::app::Action::Send(o) if o.body == lines.join("\n")));
    }

    // --- Pass 2: conversation scroll, over real rendered rows. ---

    const DANIL: &str = "danil:local:deaddrop";
    const TANISH: &str = "tanish:local:deaddrop";

    fn received(id: &str, from: &str, body: &str) -> Received {
        Received {
            id: id.into(),
            from: from.into(),
            kind: "message".into(),
            body: body.into(),
            correlation: None,
            artifacts: vec![],
            delivery: vec![],
            raw: String::new(),
        }
    }

    fn talk(messages: &[(&str, &str, &str)]) -> Snapshot {
        Snapshot {
            node: Node {
                id: "iva:local:deaddrop".into(),
                key: "ed25519:iva".into(),
                relay: "http://127.0.0.1:8787".into(),
            },
            peers: [DANIL, TANISH]
                .map(|id| Peer {
                    id: id.into(),
                    key: format!("ed25519:{id}"),
                })
                .to_vec(),
            inbox: messages
                .iter()
                .map(|(id, from, body)| received(id, from, body))
                .collect(),
            sent: vec![],
            sync: Ok(SyncSummary::default()),
        }
    }

    /// `n` short messages from Danil, numbered m1..mn.
    fn many(n: usize) -> Vec<(String, String)> {
        (1..=n)
            .map(|i| (format!("m{i}"), format!("message number {i}")))
            .collect()
    }

    fn snapshot_of(list: &[(String, String)], from: &str) -> Snapshot {
        let refs: Vec<(&str, &str, &str)> = list
            .iter()
            .map(|(id, body)| (id.as_str(), from, body.as_str()))
            .collect();
        talk(&refs)
    }

    /// Draw, then hand the drawn conversation back, as the main loop does.
    fn frame(app: &mut App, cols: u16, rows: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        let mut view = None;
        let buffer = terminal
            .draw(|f| view = draw(f, app, Glyphs::Unicode, 0).conversation)
            .unwrap()
            .buffer
            .clone();
        app.observe(view);
        (0..rows)
            .map(|y| (0..cols).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    fn open(list: &[(String, String)]) -> App {
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(snapshot_of(list, DANIL)));
        app.key(Key::Enter);
        app
    }

    fn shows(screen: &[String], text: &str) -> bool {
        screen.iter().any(|r| r.contains(text))
    }

    fn offset(app: &App) -> usize {
        let v = app.view().unwrap();
        app.scroll_offset(&v.peer, v.rows, &v.blocks)
    }

    fn bottom(app: &App) -> usize {
        let v = app.view().unwrap();
        v.total().saturating_sub(v.rows)
    }

    #[test]
    fn p2_wheel_scrolls_up_and_down_and_clamps_at_both_ends() {
        let list = many(40);
        let mut app = open(&list);
        let screen = frame(&mut app, 120, 24);
        assert!(shows(&screen, "message number 40") && !shows(&screen, "message number 1 "));
        let end = offset(&app);
        assert_eq!(end, bottom(&app));

        app.key(Key::WheelUp);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), end - WHEEL_ROWS, "a few rows, not a message");
        app.key(Key::WheelDown);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), end);
        assert!(
            app.viewport(DANIL).anchor.is_none(),
            "back at the bottom, following"
        );

        for _ in 0..200 {
            app.key(Key::WheelUp);
        }
        let screen = frame(&mut app, 120, 24);
        assert_eq!(offset(&app), 0, "clamped at the oldest row");
        assert!(shows(&screen, "message number 1 ") || shows(&screen, "message number 1\u{20}"));
        for _ in 0..200 {
            app.key(Key::WheelDown);
        }
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), bottom(&app), "clamped at the bottom");
    }

    #[test]
    fn p2_ctrl_arrows_move_half_a_view_and_page_keys_most_of_one() {
        let mut app = open(&many(60));
        frame(&mut app, 120, 24);
        let rows = app.view().unwrap().rows;
        let end = offset(&app);
        app.key(Key::ScrollUp);
        frame(&mut app, 120, 24);
        assert_eq!(end - offset(&app), rows / 2);
        app.key(Key::ScrollDown);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), end);
        app.key(Key::PageUp);
        frame(&mut app, 120, 24);
        assert_eq!(end - offset(&app), rows - 2);
        app.key(Key::PageDown);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), end);
        app.key(Key::Home);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), 0);
        app.key(Key::End);
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), end);
    }

    #[test]
    fn p2_new_messages_follow_at_the_bottom_but_never_yank_a_reader() {
        let mut list = many(40);
        let mut app = open(&list);
        frame(&mut app, 120, 24);

        // At the bottom: a new message comes into view.
        list.push(("m41".into(), "fresh at the bottom".into()));
        app.begin_refresh();
        app.finish(Ok(snapshot_of(&list, DANIL)));
        let screen = frame(&mut app, 120, 24);
        assert!(shows(&screen, "fresh at the bottom"));
        assert_eq!(app.selected_message().unwrap().id(), "m41");

        // Scrolled up: the view stays exactly where it was.
        for _ in 0..4 {
            app.key(Key::WheelUp);
        }
        let before = frame(&mut app, 120, 24);
        let top = offset(&app);
        for n in 42..=43 {
            list.push((format!("m{n}"), format!("arrived while reading {n}")));
            app.begin_refresh();
            app.finish(Ok(snapshot_of(&list, DANIL)));
        }
        let after = frame(&mut app, 120, 24);
        assert_eq!(offset(&app), top, "no jump");
        assert!(!shows(&after, "arrived while reading"));
        let body = |s: &[String]| s[4..20].to_vec();
        assert_eq!(body(&before), body(&after), "same text in view");
        assert_eq!(app.viewport(DANIL).unseen, 2);
        assert!(after[2].contains("+2 new"), "{:?}", after[2]);

        // Back to the bottom clears it.
        app.key(Key::ScrollDown);
        app.key(Key::ScrollDown);
        app.key(Key::ScrollDown);
        app.key(Key::ScrollDown);
        let screen = frame(&mut app, 120, 24);
        assert!(shows(&screen, "arrived while reading 43"));
        assert_eq!(app.viewport(DANIL).unseen, 0);
        assert!(!screen[2].contains("new"), "{:?}", screen[2]);
    }

    #[test]
    fn p2_your_own_send_snaps_to_the_bottom() {
        let list = many(40);
        let mut app = open(&list);
        frame(&mut app, 120, 24);
        for _ in 0..5 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 120, 24);
        app.key(Key::Char('h'));
        app.key(Key::Char('i'));
        let Action::Send(_) = app.key(Key::Enter) else {
            panic!("send")
        };
        app.finish_send(Ok("s1".into()));
        let mut snap = snapshot_of(&list, DANIL);
        snap.sent.push(crate::snapshot::Sent {
            id: "s1".into(),
            to: DANIL.into(),
            kind: "message".into(),
            body: "hi".into(),
            correlation: None,
            artifacts: vec![],
            delivery: vec![],
            acked_by: vec![],
            raw: String::new(),
        });
        app.begin_refresh();
        app.finish(Ok(snap));
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), bottom(&app));
        assert_eq!(app.selected_message().unwrap().id(), "s1");
    }

    #[test]
    fn p2_rows_are_counted_after_wrapping() {
        let list = vec![
            ("m1".to_owned(), "first".to_owned()),
            ("m2".to_owned(), "one\ntwo\nthree\nfour\nfive".to_owned()),
            ("m3".to_owned(), "word ".repeat(60)),
        ];
        let mut app = open(&list);
        frame(&mut app, 120, 40);
        let blocks = app.view().unwrap().blocks.clone();
        let rows = |id: &str| {
            let b = blocks.iter().find(|b| b.id == id).unwrap();
            b.end - b.start
        };
        assert_eq!(
            rows("m2"),
            5,
            "five lines, five rows (same author, no header)"
        );
        let cols = app.view().unwrap().width as usize;
        let expected = wrap(&"word ".repeat(60), cols.saturating_sub(4)).len();
        assert!(expected > 1);
        assert_eq!(rows("m3"), expected, "soft wrap counted");
        // Blocks tile the conversation with no gaps.
        for pair in blocks.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
    }

    #[test]
    fn p2_resize_keeps_the_same_text_at_the_top() {
        let list: Vec<(String, String)> = (1..=30)
            .map(|i| {
                (
                    format!("m{i}"),
                    format!("m{i}: {}", "long words wrap here ".repeat(6)),
                )
            })
            .collect();
        let mut app = open(&list);
        frame(&mut app, 140, 30);
        for _ in 0..10 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 140, 30);
        let anchor = app.viewport(DANIL).anchor.clone().unwrap();
        // Narrower: everything wraps more, the anchored message stays on top.
        let narrow = frame(&mut app, 80, 30);
        assert_eq!(app.viewport(DANIL).anchor.unwrap().message, anchor.message);
        let top_block = app
            .view()
            .unwrap()
            .blocks
            .iter()
            .find(|b| b.id == anchor.message)
            .unwrap()
            .clone();
        let off = offset(&app);
        assert!(
            (top_block.start..top_block.end).contains(&off),
            "anchored message on top"
        );
        assert!(narrow.len() == 30);
        // Tiny terminals still work.
        for (c, r) in [(40, 12), (24, 8), (10, 6)] {
            frame(&mut app, c, r);
            app.key(Key::WheelUp);
            app.key(Key::ScrollDown);
            frame(&mut app, c, r);
        }
    }

    #[test]
    fn p2_a_growing_composer_does_not_move_the_reading_position() {
        let mut app = open(&many(50));
        frame(&mut app, 120, 30);
        for _ in 0..6 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 120, 30);
        let top = offset(&app);
        let rows_before = app.view().unwrap().rows;
        for c in "line".chars() {
            app.key(Key::Char(c));
        }
        for _ in 0..5 {
            app.key(Key::Newline);
            app.key(Key::Char('x'));
        }
        frame(&mut app, 120, 30);
        assert!(
            app.view().unwrap().rows < rows_before,
            "the composer took rows"
        );
        assert_eq!(offset(&app), top, "the top of the reading position held");
        assert_eq!(app.compose.as_ref().unwrap().draft, "line\nx\nx\nx\nx\nx");
        // Scrolling while composing leaves the draft alone.
        app.key(Key::WheelUp);
        app.key(Key::ScrollDown);
        app.key(Key::Home);
        app.key(Key::End);
        assert_eq!(app.compose.as_ref().unwrap().draft, "line\nx\nx\nx\nx\nx");
    }

    #[test]
    fn p2_each_conversation_keeps_its_place() {
        let mut list: Vec<(&str, &str, String)> = (1..=40)
            .map(|i| ("", DANIL, format!("danil {i}")))
            .collect();
        list.extend((1..=40).map(|i| ("", TANISH, format!("tanish {i}"))));
        let ids: Vec<String> = (0..list.len()).map(|i| format!("x{i}")).collect();
        let msgs: Vec<(&str, &str, &str)> = list
            .iter()
            .zip(&ids)
            .map(|((_, from, body), id)| (id.as_str(), *from, body.as_str()))
            .collect();
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(talk(&msgs)));
        frame(&mut app, 120, 24);
        for _ in 0..8 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 120, 24);
        let danil_top = offset(&app);
        let danil_pick = app.selected_message().unwrap().id().to_owned();

        app.key(Key::Down); // contacts: to tanish
        frame(&mut app, 120, 24);
        assert_eq!(app.selected_peer().unwrap().id, TANISH);
        assert_eq!(offset(&app), bottom(&app), "tanish opens at the bottom");

        app.key(Key::Up); // back to danil
        frame(&mut app, 120, 24);
        assert_eq!(offset(&app), danil_top, "danil kept its place");
        assert_eq!(app.selected_message().unwrap().id(), danil_pick);
    }

    #[test]
    fn p2_selection_stays_inside_the_view() {
        let mut app = open(&many(60));
        frame(&mut app, 120, 24);
        for _ in 0..20 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 120, 24);
        let view = app.view().unwrap().clone();
        let off = offset(&app);
        let id = app.selected_message().unwrap().id().to_owned();
        let b = view.blocks.iter().find(|b| b.id == id).unwrap();
        assert!(
            b.end > off && b.start < off + view.rows,
            "selection followed the scroll"
        );

        // Moving the selection reveals it.
        for _ in 0..30 {
            app.key(Key::Up);
            frame(&mut app, 120, 24);
            let v = app.view().unwrap().clone();
            let off = offset(&app);
            let id = app.selected_message().unwrap().id().to_owned();
            let b = v.blocks.iter().find(|b| b.id == id).unwrap();
            assert!(b.end > off && b.start < off + v.rows);
        }
    }

    #[test]
    fn p2_open_on_starts_in_that_conversation() {
        let list = many(5);
        let mut app = App::new();
        app.open_on(TANISH);
        app.begin_refresh();
        let mut snap = snapshot_of(&list, DANIL);
        snap.inbox.push(received("t1", TANISH, "tanish here"));
        app.finish(Ok(snap));
        assert_eq!(app.selected_peer().unwrap().id, TANISH);
        assert_eq!(app.focus, Focus::Conversation);
        let screen = frame(&mut app, 120, 24);
        assert!(shows(&screen, "tanish here"));

        let mut app = App::new();
        app.open_on("nobody:local:deaddrop");
        app.begin_refresh();
        app.finish(Ok(snapshot_of(&list, DANIL)));
        assert!(app.status.contains("no contact nobody"));
    }

    #[test]
    fn p2_clicks_open_contacts_without_arrow_keys() {
        let mut snap = snapshot_of(&many(40), DANIL);
        snap.inbox.push(received("t1", TANISH, "tanish here"));
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(snap));
        assert_eq!(app.focus, Focus::Contacts);
        app.key(Key::ClickContact(1));
        assert_eq!(app.selected_peer().unwrap().id, TANISH);
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::ClickContact(0));
        assert_eq!(app.selected_peer().unwrap().id, DANIL);
        let screen = frame(&mut app, 120, 24);
        assert!(shows(&screen, "message number 40"));
        // The wheel works right away, no keys needed.
        app.key(Key::WheelUp);
        frame(&mut app, 120, 24);
        assert!(app.viewport(DANIL).anchor.is_some());
        app.key(Key::ClickContact(9));
        assert_eq!(app.selected_peer().unwrap().id, DANIL, "no such contact");

        // A typed draft is never dropped by a click.
        app.key(Key::Char('h'));
        app.key(Key::ClickContact(1));
        assert_eq!(app.selected_peer().unwrap().id, DANIL);
        assert_eq!(app.compose.as_ref().unwrap().draft, "h");
        app.key(Key::Backspace);
        app.key(Key::ClickContact(1));
        assert_eq!(
            app.selected_peer().unwrap().id,
            TANISH,
            "an empty draft is let go"
        );
        assert!(!app.composing());

        app.key(Key::Esc);
        assert_eq!(app.focus, Focus::Contacts);
        frame(&mut app, 120, 24);
        let v = app.view().unwrap().clone();
        // A plain click in the conversation focuses it and selects nothing.
        app.key(Key::MouseDown {
            column: v.x + 6,
            row: v.y + 2,
        });
        app.key(Key::MouseUp {
            column: v.x + 6,
            row: v.y + 2,
        });
        assert_eq!(app.focus, Focus::Conversation);
        assert!(app.selection().is_none());
    }

    #[test]
    fn p2_contact_rows_are_reported_for_clicks() {
        let mut snap = snapshot_of(&many(3), DANIL);
        snap.inbox.push(received("t1", TANISH, "x"));
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(snap));
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        let mut drawn = Drawn::default();
        let buffer = terminal
            .draw(|f| drawn = draw(f, &app, Glyphs::Unicode, 0))
            .unwrap()
            .buffer
            .clone();
        let contacts = drawn.contacts.unwrap();
        assert_eq!(contacts.rows, 2);
        let row = |y: u16| -> String { (0..60).map(|x| buffer[(x, y)].symbol()).collect() };
        assert!(row(contacts.y).contains("danil"), "{:?}", row(contacts.y));
        assert!(row(contacts.y + 1).contains("tanish"));
        assert_eq!(contacts.at(contacts.x + 3, contacts.y + 1), Some(1));
    }

    // --- Pass 2: app-native selection and copy. ---

    /// Draw, observe, and return the buffer.
    fn painted(app: &mut App, cols: u16, rows: u16) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        let mut drawn = Drawn::default();
        let buffer = terminal
            .draw(|f| drawn = draw(f, app, Glyphs::Unicode, 0))
            .unwrap()
            .buffer
            .clone();
        app.observe(drawn.conversation);
        app.observe_composer(drawn.composer);
        buffer
    }

    /// Screen cells (x of first char, x of last char, y) of the `nth`
    /// occurrence of `needle`, reading cells so wide glyphs count right.
    fn locate(buffer: &ratatui::buffer::Buffer, needle: &str, nth: usize) -> (u16, u16, u16) {
        let mut seen = 0;
        for y in 0..buffer.area.height {
            let mut text = String::new();
            let mut xs = Vec::new();
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                for c in symbol.chars() {
                    xs.push(x);
                    text.push(c);
                }
                x += width(symbol).max(1) as u16;
            }
            let mut from = 0;
            while let Some(found) = text[from..].find(needle) {
                let at = from + found;
                if seen == nth {
                    let first = text[..at].chars().count();
                    let last = first + needle.chars().count() - 1;
                    return (xs[first], xs[last], y);
                }
                seen += 1;
                from = at + needle.len();
            }
        }
        panic!("{needle:?} #{nth} not on screen");
    }

    /// Drag with the mouse from the first char of `from` to the last of `to`.
    fn drag(app: &mut App, buffer: &ratatui::buffer::Buffer, from: &str, to: &str) {
        let (x0, _, y0) = locate(buffer, from, 0);
        let (_, x1, y1) = locate(buffer, to, 0);
        app.key(Key::MouseDown {
            column: x0,
            row: y0,
        });
        app.key(Key::MouseDrag {
            column: x1,
            row: y1,
        });
        app.key(Key::MouseUp {
            column: x1,
            row: y1,
        });
    }

    fn copied(app: &mut App) -> String {
        match app.key(Key::Copy) {
            Action::Copy(text) => text,
            other => panic!("expected a copy, got {other:?}"),
        }
    }

    fn opened(messages: &[(&str, &str)]) -> App {
        let list: Vec<(String, String)> = messages
            .iter()
            .map(|(id, body)| ((*id).to_owned(), (*body).to_owned()))
            .collect();
        open(&list)
    }

    #[test]
    fn sel_a_substring_inside_one_line_copies_exactly() {
        let mut app = opened(&[("m1", "Hobotnice imaju tri srca i plavu krv.")]);
        let buffer = painted(&mut app, 120, 24);
        drag(&mut app, &buffer, "tri", "srca");
        assert_eq!(app.selected_text().unwrap(), "tri srca");

        // Only the selected text is highlighted.
        let buffer = painted(&mut app, 120, 24);
        let (x0, x1, y) = locate(&buffer, "tri srca", 0);
        for x in 0..buffer.area.width {
            let picked = buffer[(x, y)].bg == T.selection_surface;
            assert_eq!(picked, (x0..=x1).contains(&x), "column {x}");
        }
        assert_eq!(copied(&mut app), "tri srca");
        assert!(app.status.contains("copied 8 chars"));
    }

    #[test]
    fn sel_across_a_soft_wrap_copies_the_original_text() {
        let body = "Hobotnice imaju tri srca i plavu krv, a dva od njih pumpaju krv kroz skrge dok trece radi za ostatak tela";
        let mut app = opened(&[("m1", body)]);
        let buffer = painted(&mut app, 70, 24);
        let rows = &app.view().unwrap().text_rows;
        assert!(rows.iter().flatten().count() >= 2, "it wraps at this width");
        drag(&mut app, &buffer, "plavu", "skrge");
        let text = app.selected_text().unwrap();
        assert!(body.contains(&text), "{text:?} is a piece of the original");
        assert!(text.starts_with("plavu") && text.ends_with("skrge"));
        assert!(!text.contains('\n') && !text.contains('│') && !text.contains("  "));
    }

    #[test]
    fn sel_across_a_real_newline_keeps_it() {
        let mut app = opened(&[("m1", "prva linija\ndruga linija")]);
        let buffer = painted(&mut app, 120, 24);
        drag(&mut app, &buffer, "linija", "druga");
        assert_eq!(app.selected_text().unwrap(), "linija\ndruga");
    }

    #[test]
    fn sel_across_messages_joins_their_parts() {
        let mut app = opened(&[("m1", "alpha beta gamma"), ("m2", "delta epsilon")]);
        let buffer = painted(&mut app, 120, 24);
        drag(&mut app, &buffer, "gamma", "delta");
        assert_eq!(app.selected_text().unwrap(), "gamma\ndelta");
        // Backwards works the same.
        let (_, x1, y1) = locate(&buffer, "delta", 0);
        let (x0, _, y0) = locate(&buffer, "gamma", 0);
        app.key(Key::MouseDown {
            column: x1,
            row: y1,
        });
        app.key(Key::MouseUp {
            column: x0,
            row: y0,
        });
        assert_eq!(app.selected_text().unwrap(), "gamma\ndelta");
    }

    #[test]
    fn sel_unicode_and_emoji_stay_whole() {
        let mut app = opened(&[("m1", "Ovo je šđčćž i 🐙 hobotnica, kraj.")]);
        let buffer = painted(&mut app, 120, 24);
        drag(&mut app, &buffer, "šđčćž", "hobotnica");
        assert_eq!(app.selected_text().unwrap(), "šđčćž i 🐙 hobotnica");
        assert_eq!(copied(&mut app), "šđčćž i 🐙 hobotnica");
    }

    #[test]
    fn sel_narrow_terminal_and_dragging_past_the_pane() {
        let body = "jedan dva tri cetiri pet sest sedam osam devet deset jedanaest dvanaest";
        let mut app = opened(&[("m1", body)]);
        let buffer = painted(&mut app, 50, 16);
        let (x0, _, y0) = locate(&buffer, "dva", 0);
        app.key(Key::MouseDown {
            column: x0,
            row: y0,
        });
        // Over the contact column, then below the pane: only message text.
        app.key(Key::MouseDrag { column: 2, row: y0 });
        app.key(Key::MouseUp {
            column: 30,
            row: 15,
        });
        let text = app.selected_text().unwrap();
        assert!(body.contains(&text), "{text:?}");
        assert!(!text.contains("danil") && !text.contains('│'));
    }

    #[test]
    fn sel_survives_scrolling_and_copy_or_quit() {
        let mut list = many(40);
        list[37].1 = "the selected words are here".into();
        let mut app = open(&list);
        let buffer = painted(&mut app, 120, 24);
        drag(&mut app, &buffer, "selected", "words");
        // Scrolling keeps the same text selected (it is text, not cells).
        for _ in 0..10 {
            app.key(Key::WheelUp);
        }
        painted(&mut app, 120, 24);
        assert_eq!(app.selected_text().unwrap(), "selected words");
        assert_eq!(copied(&mut app), "selected words");
        assert_eq!(
            app.key(Key::Copy),
            Action::Copy("selected words".into()),
            "copying does not quit"
        );

        // Esc clears; then Ctrl+C quits as before.
        assert_eq!(app.key(Key::Esc), Action::None);
        assert!(app.selection().is_none());
        assert_eq!(
            app.focus,
            Focus::Conversation,
            "esc only cleared the selection"
        );
        assert_eq!(app.key(Key::Copy), Action::Quit);
    }

    #[test]
    fn sel_a_plain_click_selects_nothing_and_composing_survives() {
        let mut app = opened(&[("m1", "hello world")]);
        let buffer = painted(&mut app, 120, 24);
        let (x, _, y) = locate(&buffer, "world", 0);
        app.key(Key::Char('h'));
        app.key(Key::MouseDown { column: x, row: y });
        app.key(Key::MouseUp { column: x, row: y });
        assert!(app.selection().is_none());
        assert_eq!(
            app.compose.as_ref().unwrap().draft,
            "h",
            "selecting never edits the draft"
        );
        drag(&mut app, &buffer, "hello", "world");
        assert_eq!(copied(&mut app), "hello world");
        assert_eq!(app.compose.as_ref().unwrap().draft, "h");
    }

    #[test]
    fn contacts_wheel_moves_the_contact_selection_and_clamps() {
        let mut snap = snapshot_of(&many(3), DANIL);
        snap.inbox.push(received("t1", TANISH, "x"));
        let mut app = App::new();
        app.begin_refresh();
        app.finish(Ok(snap));
        app.key(Key::ClickContact(0));
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::ContactsDown);
        assert_eq!(app.selected_peer().unwrap().id, TANISH);
        assert_eq!(app.focus, Focus::Contacts, "wheel picks, a click opens");
        app.key(Key::ContactsDown);
        assert_eq!(
            app.selected_peer().unwrap().id,
            TANISH,
            "clamped at the last"
        );
        app.key(Key::ContactsUp);
        app.key(Key::ContactsUp);
        assert_eq!(
            app.selected_peer().unwrap().id,
            DANIL,
            "clamped at the first"
        );
        // A typed draft is never dropped by the wheel either.
        app.key(Key::ClickContact(0));
        app.key(Key::Char('x'));
        app.key(Key::ContactsDown);
        assert_eq!(app.selected_peer().unwrap().id, DANIL);
        assert_eq!(app.compose.as_ref().unwrap().draft, "x");
    }

    /// The pre-range `wrap`, kept verbatim to prove the new one draws the
    /// same lines.
    fn wrap_reference(text: &str, max: usize) -> Vec<String> {
        let max = max.max(1);
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if width(&candidate) <= max {
                    line = candidate;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                for c in word.chars() {
                    if width(&line) + width(c.encode_utf8(&mut [0; 4])) > max {
                        lines.push(std::mem::take(&mut line));
                    }
                    line.push(c);
                }
            }
            lines.push(line);
        }
        lines
    }

    #[test]
    fn wrap_ranges_draw_exactly_what_wrap_drew() {
        let pieces = [
            "a",
            "bb",
            " ",
            "  ",
            "\n",
            "šđ",
            "🐙",
            "word",
            "longwordwithoutbreaks",
            "x y",
        ];
        let mut seed: u64 = 7;
        for _ in 0..5000 {
            let mut text = String::new();
            for _ in 0..(seed % 12) {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push_str(pieces[(seed >> 33) as usize % pieces.len()]);
            }
            let max = (seed % 9) as usize;
            assert_eq!(
                wrap(&text, max),
                wrap_reference(&text, max),
                "{text:?} at {max}"
            );
            seed = seed.wrapping_add(3);
        }
    }

    // --- Pass 3: composer cursor editing. ---

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            app.key(if c == '\n' {
                Key::Newline
            } else {
                Key::Char(c)
            });
        }
    }

    fn draft(app: &App) -> (String, usize) {
        let c = app.compose.as_ref().unwrap();
        (c.draft.clone(), c.cursor)
    }

    fn times(app: &mut App, key: Key, n: usize) {
        for _ in 0..n {
            app.key(key);
        }
    }

    #[test]
    fn p3_left_right_move_and_typing_inserts_at_the_cursor() {
        let mut app = chatting("helo world");
        assert_eq!(draft(&app), ("helo world".into(), 10));
        times(&mut app, Key::Left, 7);
        app.key(Key::Char('l'));
        assert_eq!(draft(&app), ("hello world".into(), 4));
        times(&mut app, Key::Left, 20);
        assert_eq!(draft(&app).1, 0, "clamped at the start");
        typed(&mut app, ">> ");
        assert_eq!(draft(&app), (">> hello world".into(), 3));
        times(&mut app, Key::Right, 40);
        assert_eq!(draft(&app).1, 14, "clamped at the end");
        app.key(Key::Char('!'));
        assert_eq!(draft(&app).0, ">> hello world!");
    }

    #[test]
    fn p3_backspace_and_delete_at_the_cursor() {
        let mut app = chatting("abcdef");
        times(&mut app, Key::Left, 3);
        app.key(Key::Backspace);
        assert_eq!(draft(&app), ("abdef".into(), 2));
        app.key(Key::Delete);
        assert_eq!(draft(&app), ("abef".into(), 2));
        times(&mut app, Key::Right, 9);
        app.key(Key::Delete);
        assert_eq!(draft(&app).0, "abef", "delete at the end does nothing");
        times(&mut app, Key::Left, 9);
        app.key(Key::Backspace);
        assert_eq!(
            draft(&app),
            ("abef".into(), 0),
            "backspace at the start does nothing"
        );
    }

    #[test]
    fn p3_ctrl_j_inserts_a_newline_at_the_cursor() {
        let mut app = chatting("prvi drugi");
        times(&mut app, Key::Left, 6);
        app.key(Key::Newline);
        assert_eq!(draft(&app), ("prvi\n drugi".into(), 5));
        app.key(Key::Delete);
        assert_eq!(draft(&app).0, "prvi\ndrugi");
        let Action::Send(out) = app.key(Key::Enter) else {
            panic!("enter sends")
        };
        assert_eq!(
            out.body, "prvi\ndrugi",
            "Enter sends the whole draft, wherever the cursor is"
        );
    }

    #[test]
    fn p3_unicode_moves_and_deletes_whole_characters() {
        let mut app = chatting("šđ🐙👩‍💻e\u{301}!");
        app.key(Key::Left); // before "!"
        app.key(Key::Backspace);
        assert_eq!(draft(&app).0, "šđ🐙👩‍💻!", "the accented e as one");
        app.key(Key::Backspace);
        assert_eq!(draft(&app).0, "šđ🐙!", "the ZWJ sequence as one");
        app.key(Key::Left);
        app.key(Key::Delete);
        assert_eq!(draft(&app), ("šđ!".into(), 2), "the emoji as one");
        app.key(Key::Char('ž'));
        assert_eq!(draft(&app), ("šđž!".into(), 3));
    }

    /// The symbol drawn where the terminal cursor stands.
    fn under_cursor(app: &App, cols: u16, rows: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal
            .draw(|f| {
                draw(f, app, Glyphs::Unicode, 0);
            })
            .unwrap();
        let p = terminal.get_cursor_position().unwrap();
        terminal.backend().buffer()[(p.x, p.y)].symbol().to_owned()
    }

    #[test]
    fn p3_the_screen_cursor_follows_through_soft_wraps() {
        let text = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
        let mut app = chatting(text);
        // Walk the cursor back through every character; the next character
        // must always be drawn exactly where the terminal cursor stands.
        let chars: Vec<char> = text.chars().collect();
        for i in (0..chars.len()).rev() {
            app.key(Key::Left);
            let here = under_cursor(&app, 40, 20);
            if chars[i] == ' ' && here == " " {
                continue;
            }
            assert_eq!(here, chars[i].to_string(), "at {i}");
        }
        assert!(compose_rows(&app, 40, 20) > 1, "it did wrap");
    }

    #[test]
    fn p3_the_screen_cursor_handles_newlines_and_wide_characters() {
        let mut app = chatting("prva\n🐙 druga");
        for expected in ["a", "g", "u", "r", "d", " ", "🐙"] {
            app.key(Key::Left);
            assert_eq!(under_cursor(&app, 60, 20), expected);
        }
        app.key(Key::Left);
        // Before the 🐙 sits the newline: the cursor is at the end of "prva".
        app.key(Key::Left);
        assert_eq!(under_cursor(&app, 60, 20), "a");
    }

    #[test]
    fn p3_send_resets_the_cursor_and_failure_keeps_it() {
        let mut app = chatting("hello");
        app.key(Key::Left);
        let Action::Send(_) = app.key(Key::Enter) else {
            panic!()
        };
        app.finish_send(Err("down".into()));
        assert_eq!(
            draft(&app),
            ("hello".into(), 4),
            "failure keeps draft and cursor"
        );
        app.key(Key::Enter);
        app.finish_send(Ok("s1".into()));
        assert_eq!(draft(&app), (String::new(), 0));
        typed(&mut app, "next");
        assert_eq!(draft(&app), ("next".into(), 4));
    }

    #[test]
    fn p3_left_right_outside_the_composer_are_unchanged() {
        let mut app = chatting("x");
        app.key(Key::Esc); // closes the composer only
        assert!(!app.composing());
        assert_eq!(app.focus, Focus::Conversation);
        app.key(Key::Left);
        assert_eq!(app.focus, Focus::Contacts, "left still goes back a pane");
        app.key(Key::Right);
        assert_eq!(app.focus, Focus::Conversation, "right still opens");
    }

    // --- Pass 3 correction: clickable composer, paste. ---

    /// Click the first character of the `nth` `needle` drawn on screen.
    fn click_on(app: &mut App, needle: &str, nth: usize) -> Action {
        let buffer = painted(app, 100, 24);
        let (x, _, y) = locate(&buffer, needle, nth);
        app.key(Key::ComposerClick { column: x, row: y })
    }

    fn composer_draft(app: &App) -> (String, usize) {
        let c = app.compose.as_ref().expect("composing");
        (c.draft.clone(), c.cursor)
    }

    #[test]
    fn pc_the_human_flow_click_paste_fix_the_typo_enter() {
        let mut app = opened(&[("m1", "zdravo")]);
        app.key(Key::Esc); // back to contacts: nothing is focused for writing
        assert!(!app.composing());
        // 1. Click "drop message".
        let buffer = painted(&mut app, 100, 24);
        assert!(locate(&buffer, "drop message", 0).2 > 0);
        assert_eq!(click_on(&mut app, "drop message", 0), Action::None);
        assert!(app.composing(), "a click starts writing");
        // 2. Paste.
        app.paste("volim hobotncie 🐙");
        assert_eq!(composer_draft(&app), ("volim hobotncie 🐙".into(), 17));
        assert!(app.composing(), "a paste never sends");
        // 4. Click right on the typo's "c".
        click_on(&mut app, "cie", 0);
        assert_eq!(composer_draft(&app).1, "volim hobotn".chars().count());
        // 6. Fix: drop the c, step over the i, put the c back.
        app.key(Key::Delete);
        app.key(Key::Right);
        app.key(Key::Char('c'));
        assert_eq!(composer_draft(&app).0, "volim hobotnice 🐙");
        // 7. Only Enter sends, and it sends exactly the edited draft.
        assert_eq!(
            app.key(Key::Enter),
            Action::Send(Outgoing {
                to: DANIL.into(),
                body: "volim hobotnice 🐙".into(),
            })
        );
    }

    #[test]
    fn pc_paste_inserts_at_the_cursor_and_keeps_newlines() {
        let mut app = opened(&[("m1", "x")]);
        app.key(Key::Char('['));
        app.key(Key::Char(']'));
        app.key(Key::Left);
        app.paste("prvi red\r\ndrugi\tred\nšđ 🐙 e\u{301}");
        let (draft, cursor) = composer_draft(&app);
        assert_eq!(draft, "[prvi red\ndrugi    red\nšđ 🐙 e\u{301}]");
        assert_eq!(
            cursor,
            graphemes_of("[prvi red\ndrugi    red\nšđ 🐙 e\u{301}")
        );
        assert!(app.composing(), "multiline paste sends nothing");
        // Pasted text is ordinary draft text: editable at the cursor.
        app.key(Key::Backspace);
        assert!(
            composer_draft(&app).0.ends_with("🐙 ]"),
            "the accented e went whole"
        );
        // Stray control characters never reach the draft.
        app.paste("a\u{7}b\u{1b}c");
        assert!(composer_draft(&app).0.ends_with("abc]"));
    }

    fn graphemes_of(text: &str) -> usize {
        crate::composer::graphemes(text)
    }

    #[test]
    fn pc_paste_opens_the_composer_for_the_open_contact_only() {
        let mut app = App::new();
        app.paste("nobody to send to");
        assert!(app.compose.is_none(), "no contact: nothing happens");
        let mut app = opened(&[("m1", "x")]);
        app.key(Key::Esc);
        app.paste("hello");
        assert_eq!(composer_draft(&app), ("hello".into(), 5));
    }

    #[test]
    fn pc_clicks_reach_wrapped_rows_and_land_beside_wide_characters() {
        let mut app = opened(&[("m1", "x")]);
        app.paste("alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo");
        let buffer = painted(&mut app, 100, 24);
        assert!(compose_rows(&app, 100, 24) > 1, "it wrapped");
        // A word on the second visual row.
        let (x, _, y) = locate(&buffer, "papa", 0);
        app.key(Key::ComposerClick { column: x, row: y });
        let (draft, cursor) = composer_draft(&app);
        let before: String = draft.chars().take(cursor).collect();
        assert!(before.ends_with("oscar "), "{before:?}");
        // Far right of a row: the end of that row's text.
        app.key(Key::ComposerClick { column: 99, row: y });
        let (draft, cursor) = composer_draft(&app);
        let before: String = draft.chars().take(cursor).collect();
        assert!(draft.starts_with(&before));

        let mut app = opened(&[("m1", "x")]);
        // Letters that appear nowhere else on screen.
        app.paste("Q🐙W e\u{301}Z 👩‍💻X");
        for (needle, before) in [
            ("🐙", "Q"),
            ("W", "Q🐙"),
            ("Z", "Q🐙W e\u{301}"),
            ("X", "Q🐙W e\u{301}Z 👩‍💻"),
        ] {
            click_on(&mut app, needle, 0);
            let (draft, cursor) = composer_draft(&app);
            let got: String = {
                use unicode_segmentation::UnicodeSegmentation;
                draft.graphemes(true).take(cursor).collect()
            };
            assert_eq!(got, before, "click on {needle:?}");
        }
    }

    #[test]
    fn pc_nothing_but_enter_sends() {
        let mut app = opened(&[("m1", "x")]);
        assert_eq!(click_on(&mut app, "drop message", 0), Action::None);
        app.paste("one\ntwo\nthree");
        click_on(&mut app, "two", 0);
        app.key(Key::Newline);
        for key in [
            Key::Left,
            Key::Right,
            Key::Delete,
            Key::Backspace,
            Key::WheelUp,
        ] {
            assert_eq!(app.key(key), Action::None, "{key:?}");
        }
        assert!(app.composing());
        assert!(
            matches!(app.key(Key::Enter), Action::Send(_)),
            "Enter alone sends"
        );
    }

    #[test]
    fn pc_the_placeholder_says_drop_message() {
        let mut app = opened(&[("m1", "x")]);
        app.key(Key::Esc);
        let buffer = painted(&mut app, 100, 24);
        locate(&buffer, "drop message", 0);
        let mut empty = App::new();
        let buffer = painted(&mut empty, 100, 24);
        locate(&buffer, "pick a contact", 0);
    }

    // --- Final patch: right-click paste through a clipboard. ---

    struct FakeClipboard {
        reads: std::cell::Cell<usize>,
        content: Result<Option<String>, String>,
    }

    impl crate::clipboard::Clipboard for FakeClipboard {
        fn read_text(&self) -> Result<Option<String>, String> {
            self.reads.set(self.reads.get() + 1);
            self.content.clone()
        }
    }

    fn fake(content: Result<Option<&str>, &str>) -> FakeClipboard {
        FakeClipboard {
            reads: std::cell::Cell::new(0),
            content: content.map(|c| c.map(str::to_owned)).map_err(str::to_owned),
        }
    }

    /// What main does: a right click asks; the clipboard is read once; the
    /// result goes to the app.
    fn right_click(app: &mut App, clipboard: &FakeClipboard) -> Action {
        let action = app.key(Key::ComposerRightClick);
        if action == Action::ReadClipboard {
            app.pasted(crate::clipboard::Clipboard::read_text(clipboard));
        }
        action
    }

    #[test]
    fn rc_right_click_focuses_reads_once_and_never_sends() {
        let mut app = opened(&[("m1", "x")]);
        app.key(Key::Esc);
        assert!(!app.composing());
        let clip = fake(Ok(Some("volim hobotncie 🐙")));
        assert_eq!(
            right_click(&mut app, &clip),
            Action::ReadClipboard,
            "not a send"
        );
        assert_eq!(clip.reads.get(), 1);
        assert_eq!(composer_draft(&app), ("volim hobotncie 🐙".into(), 17));
        assert!(app.composing(), "remains unsent and editable");
        // Fix the typo by clicking it, then only Enter sends.
        click_on(&mut app, "cie", 0);
        app.key(Key::Delete);
        app.key(Key::Right);
        app.key(Key::Char('c'));
        assert_eq!(
            app.key(Key::Enter),
            Action::Send(Outgoing {
                to: DANIL.into(),
                body: "volim hobotnice 🐙".into(),
            })
        );
    }

    #[test]
    fn rc_inserts_at_the_cursor_keeping_the_draft_around_it() {
        let mut app = opened(&[("m1", "x")]);
        typed(&mut app, "pre post");
        times(&mut app, Key::Left, 4);
        right_click(&mut app, &fake(Ok(Some("[šđ 🐙 e\u{301}\nred dva]"))));
        assert_eq!(composer_draft(&app).0, "pre [šđ 🐙 e\u{301}\nred dva]post");
        assert!(app.composing(), "a pasted newline is text, not Enter");
        // CRLF from Windows becomes a plain newline.
        right_click(&mut app, &fake(Ok(Some("\r\nx"))));
        assert_eq!(
            composer_draft(&app).0,
            "pre [šđ 🐙 e\u{301}\nred dva]\nxpost"
        );
    }

    #[test]
    fn rc_a_failed_or_empty_clipboard_changes_nothing() {
        let mut app = opened(&[("m1", "x")]);
        typed(&mut app, "keep me");
        times(&mut app, Key::Left, 3);
        let before = composer_draft(&app);
        right_click(&mut app, &fake(Err("clipboard read timed out")));
        assert_eq!(composer_draft(&app), before);
        assert!(app.status.contains("clipboard read timed out"));
        right_click(&mut app, &fake(Ok(None)));
        assert_eq!(composer_draft(&app), before);
    }

    #[test]
    fn rc_no_contact_no_read() {
        let mut app = App::new();
        let clip = fake(Ok(Some("x")));
        assert_eq!(right_click(&mut app, &clip), Action::None);
        assert_eq!(clip.reads.get(), 0);
    }

    #[test]
    fn rc_ctrl_v_and_selection_are_unchanged() {
        let mut app = opened(&[("m1", "Hobotnice imaju tri srca.")]);
        app.paste("ctrl v");
        assert_eq!(composer_draft(&app).0, "ctrl v");
        let buffer = painted(&mut app, 100, 24);
        drag(&mut app, &buffer, "tri", "srca");
        assert_eq!(copied(&mut app), "tri srca");
        assert_eq!(
            composer_draft(&app).0,
            "ctrl v",
            "copying never touches the draft"
        );
    }

    // --- Agent Bus V0: rooms in the rail, the room stream, the wire. ---

    fn room_app() -> App {
        use deaddrop_room::{Kind, RoomConfig, RoomMessage, Status};
        let carrier =
            |id: &str, kind, mentions: &[&str], reply_to: Option<&str>, status, text: &str| {
                RoomMessage {
                    room: "d34ddr0p".into(),
                    id: id.into(),
                    kind,
                    hop: 0,
                    mentions: mentions.iter().map(|m| (*m).to_owned()).collect(),
                    reply_to: reply_to.map(str::to_owned),
                    status,
                    text: text.into(),
                }
                .encode()
            };
        let mut snap = snapshot_of(&many(2), DANIL);
        // Your request, fanned out to two members: two deliveries, one message.
        for (n, to) in [DANIL, TANISH].iter().enumerate() {
            snap.sent.push(crate::snapshot::Sent {
                id: format!("out{n}"),
                to: (*to).into(),
                kind: "message".into(),
                body: carrier(
                    "rq1",
                    Kind::Request,
                    &[TANISH],
                    None,
                    None,
                    "@tanish find X",
                ),
                correlation: None,
                artifacts: vec![],
                delivery: vec![],
                acked_by: if n == 0 { vec![(*to).into()] } else { vec![] },
                raw: String::new(),
            });
        }
        snap.inbox.push(received(
            "in1",
            TANISH,
            &carrier(
                "rp1",
                Kind::Report,
                &[],
                Some("rq1"),
                Some(Status::Ok),
                "found X",
            ),
        ));
        snap.inbox.push(received(
            "in2",
            DANIL,
            &carrier(
                "rp2",
                Kind::Report,
                &[],
                Some("rq9"),
                Some(Status::Refused),
                "iva may not ask danil",
            ),
        ));
        let mut app = App::new();
        app.set_rooms(vec![RoomConfig {
            name: "d34ddr0p".into(),
            members: vec!["iva:local:deaddrop".into(), DANIL.into(), TANISH.into()],
        }]);
        app.begin_refresh();
        app.finish(Ok(snap));
        app
    }

    fn drawn_screen(app: &mut App) -> (Vec<String>, Drawn) {
        let mut terminal = Terminal::new(TestBackend::new(140, 26)).unwrap();
        let mut drawn = Drawn::default();
        let buffer = terminal
            .draw(|f| drawn = draw(f, app, Glyphs::Unicode, 0))
            .unwrap()
            .buffer
            .clone();
        app.observe(drawn.conversation.clone());
        let rows = (0..26)
            .map(|y| (0..140).map(|x| buffer[(x, y)].symbol()).collect())
            .collect();
        (rows, drawn)
    }

    #[test]
    fn bus_the_rail_has_rooms_then_peers_and_rooms_are_clickable() {
        let mut app = room_app();
        let (rows, drawn) = drawn_screen(&mut app);
        let rail: Vec<String> = rows.iter().map(|r| r.chars().take(26).collect()).collect();
        let at = |t: &str| {
            rail.iter()
                .position(|r| r.contains(t))
                .unwrap_or_else(|| panic!("{t} in {rail:#?}"))
        };
        assert!(
            at("rooms") < at("# d34ddr0p")
                && at("# d34ddr0p") < at("peers")
                && at("peers") < at("danil")
        );
        let contacts = drawn.contacts.unwrap();
        assert_eq!(
            contacts.room_at(contacts.x + 3, at("# d34ddr0p") as u16),
            Some(0)
        );
        assert_eq!(contacts.at(contacts.x + 3, at("tanish") as u16), Some(1));
        app.key(Key::ClickRoom(0));
        assert_eq!(app.channel().as_deref(), Some("#d34ddr0p"));
    }

    #[test]
    fn bus_the_room_stream_shows_each_message_once_with_its_real_author() {
        let mut app = room_app();
        app.key(Key::ClickRoom(0));
        let (rows, drawn) = drawn_screen(&mut app);
        let screen = rows.join("\n");
        assert!(
            rows[2].contains("# d34ddr0p") && rows[2].contains("λ wire"),
            "{:?}",
            rows[2]
        );
        assert_eq!(
            screen.matches("@tanish find X").count(),
            1,
            "two deliveries, shown once"
        );
        assert!(screen.contains("tanish") && screen.contains("found X"));
        assert!(
            screen.contains("1/2"),
            "receipts: one of two deliveries acknowledged"
        );
        assert!(screen.contains("refused"), "a refusal is visible as one");
        assert!(drawn.wire_toggle.is_some());
        // The DM with danil holds only direct messages.
        assert!(app.thread(DANIL).iter().all(|r| r.room().is_none()));
        assert_eq!(app.thread(DANIL).len(), 2);
    }

    #[test]
    fn bus_the_wire_shows_real_routing_and_toggles_by_click() {
        let mut app = room_app();
        app.key(Key::ClickRoom(0));
        let (_, drawn) = drawn_screen(&mut app);
        let label = drawn.wire_toggle.unwrap();
        let key = crate::input::translate_mouse(
            ratatui::crossterm::event::MouseEvent {
                kind: ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left,
                ),
                column: label.x + 1,
                row: label.y,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            },
            &drawn,
        );
        assert_eq!(key, Some(Key::ToggleWire));
        app.key(Key::ToggleWire);
        let (rows, drawn) = drawn_screen(&mut app);
        let screen = rows.join("\n");
        assert!(screen.contains("0xd34ddr0p::wire"));
        assert!(screen.contains("you → tanish"), "{screen}");
        assert!(screen.contains("agent_request  rq1  receipts 1/2"));
        assert!(screen.contains("tanish → #d34ddr0p"));
        assert!(screen.contains("agent_report"));
        assert!(screen.contains("request_refused"));
        let wire = drawn.conversation.expect("the wire scrolls");
        assert_eq!(wire.peer, "#d34ddr0p::wire", "its own scroll state");
        assert!(
            wire.text_rows.iter().all(Option::is_none),
            "read-only: nothing to select"
        );
        app.key(Key::ToggleWire);
        assert!(!app.wire_open);
    }

    #[test]
    fn bus_authors_have_identity_colors_and_a_handoff_reads_as_one() {
        let mut app = room_app();
        let handoff = deaddrop_room::RoomMessage {
            room: "d34ddr0p".into(),
            id: "ask1".into(),
            kind: deaddrop_room::Kind::Request,
            hop: 1,
            mentions: vec![DANIL.into()],
            reply_to: Some("rq1".into()),
            status: None,
            text: "request:: github.inspect\n→ @danil\nrepo:: o/r\npath:: a.rs".into(),
        }
        .encode();
        let mut snap = app.snapshot.clone().unwrap();
        snap.inbox.push(received("in3", TANISH, &handoff));
        app.begin_refresh();
        app.finish(Ok(snap));
        app.key(Key::ClickRoom(0));
        let buffer = painted(&mut app, 140, 30);
        assert_ne!(T.author(TANISH), T.author(DANIL));
        // Color means who: each author name in its own color, bodies plain.
        assert_eq!(fg_at(&buffer, "tanish", 1), T.author(TANISH));
        assert_eq!(fg_of(&buffer, "found X"), T.text);
        // The handoff: fields muted, its target in the target's color.
        assert_eq!(fg_of(&buffer, "request::"), T.muted);
        assert_eq!(fg_of(&buffer, "github.inspect"), T.text);
        assert_eq!(fg_of(&buffer, "@danil"), T.author(DANIL));
        assert_eq!(fg_of(&buffer, "repo::"), T.muted);
        assert_eq!(fg_of(&buffer, "o/r"), T.text);
        app.key(Key::ToggleWire);
        let (rows, _) = drawn_screen(&mut app);
        let screen = rows.join("\n");
        assert!(screen.contains("tanish → danil"), "{screen}");
        assert!(
            screen.contains("agent_request  ask1  hop 1 · github.inspect · for rq1"),
            "{screen}"
        );
    }

    // --- /task::wire: provenance of what is shown. ---

    const RESEARCH: &str = "research:agent:deaddrop";
    const GITHUB: &str = "github:agent:deaddrop";
    const KLODIK: &str = "klodik:agent:deaddrop";
    const TASK_TEXT: &str = "check the release and the commit";

    /// A room with the three workers, tasks allowed to ask all of them.
    fn task_app() -> (App, Snapshot) {
        let mut snap = talk(&[]);
        snap.peers = [RESEARCH, GITHUB, KLODIK]
            .map(|id| Peer {
                id: id.into(),
                key: format!("ed25519:{id}"),
            })
            .to_vec();
        let mut app = App::new();
        app.set_rooms(vec![deaddrop_room::RoomConfig {
            name: "d34ddr0p".into(),
            members: vec![
                "iva:local:deaddrop".into(),
                RESEARCH.into(),
                GITHUB.into(),
                KLODIK.into(),
            ],
        }]);
        app.task_policy = vec![RESEARCH.into(), GITHUB.into(), KLODIK.into()];
        app.begin_refresh();
        app.finish(Ok(snap.clone()));
        app.key(Key::ClickRoom(0));
        (app, snap)
    }

    fn proposed() -> deaddrop_task::plan::Proposed {
        let step = |id: &str, capability: &str, objective: &str, deps: &[&str]| {
            deaddrop_task::plan::ProposedStep {
                id: id.into(),
                capability: capability.into(),
                objective: objective.into(),
                depends_on: deps.iter().map(|d| (*d).to_owned()).collect(),
            }
        };
        deaddrop_task::plan::Proposed {
            steps: vec![
                step("release", "web.search", "latest Rust release?", &[]),
                step(
                    "commit",
                    "github.inspect",
                    "does o/r@154f992cc3e88f51a0b6bdbf42998e94c3aeedaf modify a.rs?",
                    &[],
                ),
                step("summary", "synthesize", "summarize", &["release", "commit"]),
            ],
        }
    }

    /// Every task message, as this node's stored sent messages.
    fn store(snap: &mut Snapshot, sends: &[crate::app::RoomOutgoing]) {
        for (n, o) in sends.iter().enumerate() {
            snap.sent.push(crate::snapshot::Sent {
                id: format!("{}-{n}", o.id),
                to: o.to[0].clone(),
                kind: "message".into(),
                body: o.body.clone(),
                correlation: None,
                artifacts: vec![],
                delivery: vec![],
                acked_by: vec![],
                raw: String::new(),
            });
        }
    }

    /// The author label above the first row showing `needle`.
    fn label_of(rows: &[String], needle: &str) -> String {
        let at = rows
            .iter()
            .position(|r| r.contains(needle))
            .unwrap_or_else(|| panic!("{needle} in {rows:#?}"));
        rows[..at]
            .iter()
            .rev()
            .map(|r| r.split('│').nth(1).unwrap_or("").trim().to_owned())
            .find(|t| {
                t == "you"
                    || t.starts_with("λ wire")
                    || ["research", "github", "klodik"]
                        .iter()
                        .any(|a| t.ends_with(a))
            })
            .unwrap_or_default()
    }

    #[test]
    fn task_provenance_you_only_for_what_you_wrote() {
        let (mut app, mut snap) = task_app();
        for c in TASK_TEXT.chars() {
            app.key(Key::Char(c));
        }
        app.key(Key::Newline);
        for c in "/task::wire".chars() {
            app.key(Key::Char(c));
        }
        let Action::PlanTask(request) = app.key(Key::Enter) else {
            panic!("{}", app.status)
        };
        app.planned(&request.id, Ok(proposed()));
        let sends = app.take_task_sends();
        store(&mut snap, &sends);
        // GitHub reports on its step: a worker, under its own name.
        let github_step = sends
            .iter()
            .filter_map(|o| deaddrop_room::RoomMessage::decode(&o.body)?.ok())
            .find(|m| m.mentions == [GITHUB])
            .expect("a github step");
        let report = deaddrop_room::RoomMessage {
            room: "d34ddr0p".into(),
            id: "rp-github".into(),
            kind: deaddrop_room::Kind::Report,
            hop: 0,
            mentions: vec![],
            reply_to: Some(github_step.id.clone()),
            status: Some(deaddrop_room::Status::Ok),
            text: "yes, it modifies a.rs".into(),
        };
        snap.inbox
            .push(received("in-github", GITHUB, &report.encode()));
        app.begin_refresh();
        app.finish(Ok(snap));
        let rows = frame(&mut app, 140, 60);
        assert_eq!(label_of(&rows, TASK_TEXT), "you", "the human's own words");
        assert_eq!(label_of(&rows, "/task::wire"), "you");
        assert!(label_of(&rows, "task:: accepted").starts_with("λ wire"));
        assert!(
            label_of(&rows, "latest Rust release?").starts_with("λ wire"),
            "a step request"
        );
        assert!(label_of(&rows, "request:: github.inspect").starts_with("λ wire"));
        assert!(
            label_of(&rows, "yes, it modifies a.rs").ends_with("github"),
            "real author"
        );
        // `you` labels nothing the wire wrote.
        for o in &sends[1..] {
            let m = deaddrop_room::RoomMessage::decode(&o.body)
                .unwrap()
                .unwrap();
            let first = m.text.lines().next().unwrap();
            assert_ne!(label_of(&rows, first), "you", "{first}");
        }
        // The wire view names it the same way.
        app.key(Key::ToggleWire);
        let wire = frame(&mut app, 140, 60).join("\n");
        assert!(wire.contains("λ wire → research"), "{wire}");
        assert!(!wire.contains("you → research"));
    }

    #[test]
    fn task_provenance_local_notes_and_planner_are_the_wire() {
        let (mut app, _) = task_app();
        for c in "x\n/task::wire --dry-run".chars() {
            app.key(if c == '\n' {
                Key::Newline
            } else {
                Key::Char(c)
            });
        }
        let Action::PlanTask(request) = app.key(Key::Enter) else {
            panic!("{}", app.status)
        };
        let rows = frame(&mut app, 140, 30);
        assert!(
            label_of(&rows, "planning").starts_with("λ wire"),
            "planner status"
        );
        app.planned(&request.id, Ok(proposed()));
        let rows = frame(&mut app, 140, 30);
        assert!(label_of(&rows, "task:: dry-run").starts_with("λ wire"));
        assert!(
            !rows
                .iter()
                .any(|r| r.split('│').nth(1).unwrap_or("").trim() == "you")
        );
    }

    #[test]
    fn task_provenance_from_an_earlier_session_and_typed_by_hand() {
        let (_, mut snap) = task_app();
        let msg = |id: &str, text: &str| crate::snapshot::Sent {
            id: id.into(),
            to: RESEARCH.into(),
            kind: "message".into(),
            body: deaddrop_room::RoomMessage {
                room: "d34ddr0p".into(),
                id: id.into(),
                kind: deaddrop_room::Kind::Message,
                hop: 0,
                mentions: vec![],
                reply_to: None,
                status: None,
                text: text.into(),
            }
            .encode(),
            correlation: None,
            artifacts: vec![],
            delivery: vec![],
            acked_by: vec![],
            raw: String::new(),
        };
        // Stored before this session: the wire's lifecycle line.
        snap.sent
            .push(msg("old", "task:: complete\nid:: T-000001\nresult:: done"));
        let mut app = App::new();
        app.set_rooms(vec![deaddrop_room::RoomConfig {
            name: "d34ddr0p".into(),
            members: vec!["iva:local:deaddrop".into(), RESEARCH.into()],
        }]);
        app.begin_refresh();
        app.finish(Ok(snap.clone()));
        // This session, typed by the person: theirs, whatever it says.
        snap.sent
            .push(msg("typed", "task:: this is just me talking"));
        app.begin_refresh();
        app.finish(Ok(snap));
        app.key(Key::ClickRoom(0));
        let rows = frame(&mut app, 140, 30);
        assert!(label_of(&rows, "task:: complete").starts_with("λ wire"));
        assert_eq!(label_of(&rows, "just me talking"), "you");
    }

    // --- Agent Bus V0: room scrolling (same engine as DMs). ---

    /// A room with `n` long, wrapping Research-style reports.
    fn long_room(n: usize) -> (App, Snapshot) {
        let mut snap = snapshot_of(&many(3), DANIL);
        for i in 0..n {
            snap.inbox
                .push(received(&format!("rr{i}"), TANISH, &report(i)));
        }
        let mut app = App::new();
        app.set_rooms(vec![deaddrop_room::RoomConfig {
            name: "d34ddr0p".into(),
            members: vec!["iva:local:deaddrop".into(), DANIL.into(), TANISH.into()],
        }]);
        app.begin_refresh();
        app.finish(Ok(snap.clone()));
        app.key(Key::ClickRoom(0));
        (app, snap)
    }

    fn report(i: usize) -> String {
        deaddrop_room::RoomMessage {
            room: "d34ddr0p".into(),
            id: format!("report{i}"),
            kind: deaddrop_room::Kind::Report,
            hop: 0,
            mentions: vec![],
            reply_to: Some("q".into()),
            status: Some(deaddrop_room::Status::Ok),
            text: format!(
                "Report {i} begins here. {} Sources:\n1. Announcing Rust 1.99.0\n   https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/ end {i}",
                "The latest stable release is documented on the official blog. ".repeat(4)
            ),
        }
        .encode()
    }

    #[test]
    fn room_wheel_scrolls_the_room_itself_and_clamps() {
        let (mut app, _) = long_room(8);
        let bottom_screen = frame(&mut app, 100, 24);
        assert!(shows(&bottom_screen, "end 7"));
        assert!(!shows(&bottom_screen, "Report 0 begins"));
        let end = offset(&app);
        app.key(Key::WheelUp);
        let up = frame(&mut app, 100, 24);
        assert_eq!(offset(&app), end - WHEEL_ROWS);
        assert_ne!(up, bottom_screen, "the room stream itself moved");
        app.key(Key::ScrollUp);
        frame(&mut app, 100, 24);
        assert!(offset(&app) < end - WHEEL_ROWS, "ctrl+up moves further");
        for _ in 0..500 {
            app.key(Key::WheelUp);
        }
        let top = frame(&mut app, 100, 24);
        assert_eq!(offset(&app), 0);
        assert!(
            shows(&top, "Report 0 begins here"),
            "the beginning is readable"
        );
        for _ in 0..500 {
            app.key(Key::WheelDown);
        }
        let back = frame(&mut app, 100, 24);
        assert!(shows(&back, "end 7"));
        assert!(
            app.viewport("#d34ddr0p").anchor.is_none(),
            "following again"
        );
    }

    #[test]
    fn room_rows_are_counted_after_wrapping_at_any_width() {
        for cols in [60, 100, 140] {
            let (mut app, _) = long_room(3);
            frame(&mut app, cols, 24);
            let view = app.view().unwrap().clone();
            assert_eq!(view.peer, "#d34ddr0p");
            let tall = view.blocks.iter().map(|b| b.end - b.start).max().unwrap();
            assert!(tall > 4, "a long report spans many rows at {cols}");
        }
    }

    #[test]
    fn room_keeps_its_place_across_a_peer_visit_and_never_yanks() {
        let (mut app, mut snap) = long_room(8);
        frame(&mut app, 100, 24);
        for _ in 0..6 {
            app.key(Key::WheelUp);
        }
        frame(&mut app, 100, 24);
        let room_top = offset(&app);
        // Visit a peer, then come back.
        app.key(Key::ClickContact(0));
        frame(&mut app, 100, 24);
        assert_eq!(app.channel().as_deref(), Some(DANIL));
        app.key(Key::ClickRoom(0));
        frame(&mut app, 100, 24);
        assert_eq!(offset(&app), room_top, "the room kept its place");
        // A new reply arrives while scrolled up: no jump, counted.
        snap.inbox.push(received("rr-new", TANISH, &report(99)));
        app.begin_refresh();
        app.finish(Ok(snap.clone()));
        let screen = frame(&mut app, 100, 24);
        assert_eq!(offset(&app), room_top, "no yank");
        assert!(screen[2].contains("+1 new"), "{:?}", screen[2]);
        for _ in 0..500 {
            app.key(Key::WheelDown);
        }
        let screen = frame(&mut app, 100, 24);
        assert_eq!(offset(&app), bottom(&app), "back at the bottom");
        assert!(shows(&screen, "Report 99 begins"), "{screen:#?}");
        assert!(!screen[2].contains("new"), "cleared at the bottom");
    }

    #[test]
    fn dm_scrolling_is_unchanged_with_rooms_present() {
        let (mut app, _) = long_room(8);
        let mut snap = app.snapshot.clone().unwrap();
        for i in 0..40 {
            snap.inbox.push(received(
                &format!("dm{i}"),
                DANIL,
                &format!("direct message {i}"),
            ));
        }
        app.begin_refresh();
        app.finish(Ok(snap));
        app.key(Key::ClickContact(0));
        frame(&mut app, 100, 24);
        let end = offset(&app);
        app.key(Key::WheelUp);
        frame(&mut app, 100, 24);
        assert_eq!(offset(&app), end - WHEEL_ROWS);
        assert!(
            app.viewport("#d34ddr0p").anchor.is_none(),
            "the room was not touched"
        );
    }

    #[test]
    fn the_wire_view_scrolls_on_its_own() {
        let (mut app, _) = long_room(40);
        app.key(Key::ToggleWire);
        let bottom = frame(&mut app, 100, 24);
        assert!(
            shows(&bottom, "report39"),
            "newest event at the bottom: {bottom:#?}"
        );
        assert!(!shows(&bottom, "0xd34ddr0p::wire") || !shows(&bottom, "report0 "));
        for _ in 0..200 {
            app.key(Key::WheelUp);
        }
        let top = frame(&mut app, 100, 24);
        assert!(
            shows(&top, "0xd34ddr0p::wire") && shows(&top, "report0 "),
            "the start is readable"
        );
        // Its scroll state is its own: the human stream did not move.
        assert!(app.viewport("#d34ddr0p").anchor.is_none());
        app.key(Key::ToggleWire);
        frame(&mut app, 100, 24);
        assert!(app.viewport("#d34ddr0p").anchor.is_none());
        let _ = top;
    }

    // --- Latency pass: truthful waiting and the sync pace. ---

    fn asked(app: &mut App, to_acked: bool) -> Snapshot {
        use deaddrop_room::{Kind, RoomMessage};
        let body = RoomMessage {
            room: "d34ddr0p".into(),
            id: "ask1".into(),
            kind: Kind::Request,
            hop: 0,
            mentions: vec![DANIL.into()],
            reply_to: None,
            status: None,
            text: "@danil please look".into(),
        }
        .encode();
        let mut snap = app.snapshot.clone().unwrap();
        snap.sent.push(crate::snapshot::Sent {
            id: "d1".into(),
            to: DANIL.into(),
            kind: "message".into(),
            body,
            correlation: None,
            artifacts: vec![],
            delivery: vec![],
            acked_by: if to_acked { vec![DANIL.into()] } else { vec![] },
            raw: String::new(),
        });
        app.begin_refresh();
        app.finish(Ok(snap.clone()));
        snap
    }

    fn report_from(from: &str, status: deaddrop_room::Status, text: &str) -> Received {
        let body = deaddrop_room::RoomMessage {
            room: "d34ddr0p".into(),
            id: format!("rep-{from}"),
            kind: deaddrop_room::Kind::Report,
            hop: 0,
            mentions: vec![],
            reply_to: Some("ask1".into()),
            status: Some(status),
            text: text.into(),
        }
        .encode();
        received(&format!("in-{from}"), from, &body)
    }

    #[test]
    fn wait_shows_receipt_then_reply_never_more() {
        let (mut app, _) = long_room(0);
        asked(&mut app, false);
        let screen = frame(&mut app, 120, 24).join("\n");
        assert!(screen.contains("@danil · sent"), "{screen}");
        let mut snap = asked(&mut app, true);
        let screen = frame(&mut app, 120, 24).join("\n");
        assert!(
            screen.contains("@danil · received · awaiting reply"),
            "{screen}"
        );
        assert!(
            !screen.contains("searching") && !screen.contains("%"),
            "no invented progress"
        );
        // The real reply reconciles the wait.
        snap.inbox.push(report_from(
            DANIL,
            deaddrop_room::Status::Ok,
            "done looking",
        ));
        app.begin_refresh();
        app.finish(Ok(snap));
        let screen = frame(&mut app, 120, 24).join("\n");
        assert!(screen.contains("done looking"));
        assert!(!screen.contains("awaiting reply"), "the wait is gone");
        assert!(app.pending().is_empty());
    }

    #[test]
    fn a_refusal_ends_the_wait_and_shows_as_refusal() {
        let (mut app, _) = long_room(0);
        let mut snap = asked(&mut app, true);
        snap.inbox.push(report_from(
            DANIL,
            deaddrop_room::Status::Refused,
            "iva may not ask danil",
        ));
        app.begin_refresh();
        app.finish(Ok(snap));
        let screen = frame(&mut app, 120, 24).join("\n");
        assert!(screen.contains("refused") && !screen.contains("awaiting reply"));
    }

    #[test]
    fn sync_is_quick_only_while_a_reply_is_awaited() {
        use std::time::{Duration, Instant};
        let (mut app, _) = long_room(0);
        let t0 = Instant::now();
        app.synced_at(t0);
        assert!(!app.awaiting(t0));
        assert!(
            !app.begin_auto_sync(t0 + Duration::from_millis(400)),
            "idle: the slow pace"
        );
        asked(&mut app, true);
        app.synced_at(t0);
        assert!(app.awaiting(t0), "an unanswered request");
        assert!(
            app.begin_auto_sync(t0 + crate::app::FAST_SYNC),
            "awaited: the quick pace"
        );
    }

    #[test]
    fn the_wait_line_animates_with_typing_dots_only() {
        let (mut app, _) = long_room(0);
        asked(&mut app, true);
        let line_at = |app: &App, glyphs: Glyphs, tick: usize| -> String {
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            let buffer = terminal
                .draw(|f| {
                    draw(f, app, glyphs, tick);
                })
                .unwrap()
                .buffer
                .clone();
            (0..24)
                .map(|y| {
                    (0..120)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .find(|r| r.contains("· awaiting reply"))
                .expect("a wait line")
        };
        let a = line_at(&app, Glyphs::Unicode, 0);
        let b = line_at(&app, Glyphs::Unicode, 6);
        assert!(
            a.contains("[·  ] @danil · received · awaiting reply"),
            "{a}"
        );
        assert!(
            b.contains("[···] @danil · received · awaiting reply"),
            "{b}"
        );
        let ascii = line_at(&app, Glyphs::Ascii, 6);
        assert!(
            ascii.contains("[...] @danil · received · awaiting reply"),
            "{ascii}"
        );
        assert!(!a.contains('%'), "no invented progress");
    }
}
