//! Rendering. Reads [`App`] and draws it; never touches the network.
//!
//! Layout, left to right like a file manager: contacts, the conversation
//! with the selected contact, and the vault — what the selected message is
//! made of. Regions are separated by hairlines and spacing, not boxes.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::{Activity, App, Focus, Row, Section};
use crate::avatar::Glyphs;
use crate::snapshot::Peer;

const ACCENT: Color = Color::Cyan;
const ALERT: Color = Color::Yellow;
const GOOD: Color = Color::Green;
const BAD: Color = Color::Red;

/// Narrower than this, the vault replaces the contacts column when focused.
const WIDE: u16 = 100;

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

/// Display width of a string, as the terminal will lay it out.
fn width(text: &str) -> usize {
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

/// Left and right content on one line, padded apart.
fn spread(left: Vec<Span<'static>>, right: Vec<Span<'static>>, cols: usize) -> Line<'static> {
    let used: usize = left.iter().chain(&right).map(Span::width).sum();
    let mut spans = left;
    spans.push(Span::raw(" ".repeat(cols.saturating_sub(used))));
    spans.extend(right);
    Line::from(spans)
}

pub fn draw(frame: &mut Frame, app: &App, glyphs: Glyphs, tick: usize) {
    let [header, rule_top, body, rule_bottom, compose, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, header, app, glyphs, tick);
    draw_rule(frame, rule_top, glyphs);
    draw_body(frame, body, app, glyphs);
    draw_rule(frame, rule_bottom, glyphs);
    draw_compose(frame, compose, app, glyphs);
    draw_footer(frame, footer, app, glyphs);
}

fn draw_rule(frame: &mut Frame, area: Rect, glyphs: Glyphs) {
    let line = glyphs.rule().repeat(area.width as usize);
    frame.render_widget(Paragraph::new(line).style(dim()), area);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs, tick: usize) {
    let mut left = vec![Span::raw(" deaddrop ").bold().fg(ACCENT), Span::raw("  ")];
    if let Some(s) = &app.snapshot {
        let (who, rest) = name(&s.node.id);
        left.extend([
            Span::raw(format!("{} ", glyphs.avatar(&s.node.id))),
            Span::raw(who.to_owned()).bold(),
            Span::styled(rest.to_owned(), dim()),
            Span::raw("   "),
        ]);
    }
    left.extend(match app.relay_ok {
        Some(true) => vec![
            Span::raw(glyphs.relay_up()).fg(GOOD),
            Span::styled(" relay", dim()),
        ],
        Some(false) => vec![
            Span::raw(glyphs.relay_down()).fg(BAD),
            Span::raw(" relay down").fg(BAD),
        ],
        None => vec![Span::styled(format!("{} relay", glyphs.idle()), dim())],
    });
    let mut counts = vec![format!("{} peers", app.peers().len())];
    if app.unread_total() > 0 {
        counts.push(format!("{} new", app.unread_total()));
    }
    if app.awaiting_total() > 0 {
        counts.push(format!("{} awaiting ack", app.awaiting_total()));
    }
    left.push(Span::styled(format!("   {}", counts.join(" · ")), dim()));

    let right = match app.activity {
        Activity::Busy(label) => vec![
            Span::raw(glyphs.spinner(tick)).fg(ACCENT),
            Span::styled(format!(" {label} "), dim()),
        ],
        Activity::Idle => vec![],
    };
    frame.render_widget(
        Paragraph::new(spread(left, right, area.width as usize)),
        area,
    );
}

fn draw_body(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
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

    if show_contacts {
        draw_contacts(frame, next.next().unwrap(), app, glyphs);
        draw_separator(frame, next.next().unwrap(), glyphs);
    }
    draw_conversation(frame, next.next().unwrap(), app, glyphs);
    if show_vault {
        draw_separator(frame, next.next().unwrap(), glyphs);
        draw_vault(frame, next.next().unwrap(), app, glyphs);
    }
}

fn draw_separator(frame: &mut Frame, area: Rect, glyphs: Glyphs) {
    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(glyphs.divider()))
        .collect();
    frame.render_widget(Paragraph::new(lines).style(dim()), area);
}

/// Inner area with one column of breathing room on each side.
fn padded(area: Rect) -> Rect {
    Rect {
        x: area.x + 1,
        width: area.width.saturating_sub(2),
        ..area
    }
}

fn heading(text: &str, focused: bool) -> Span<'static> {
    if focused {
        Span::raw(text.to_owned()).bold().fg(ACCENT)
    } else {
        Span::styled(text.to_owned(), dim())
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

fn draw_contacts(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let area = padded(area);
    let focused = app.focus == Focus::Contacts;
    let cols = area.width as usize;
    let mut lines = vec![Line::from(heading("contacts", focused)), Line::default()];

    if app.snapshot.is_some() && app.peers().is_empty() {
        lines.push(Line::styled("no one yet", dim()));
    }
    let names: Vec<&str> = app.peers().iter().map(|p| name(&p.id).0).collect();
    for (i, peer) in app.peers().iter().enumerate() {
        let selected = i == app.contact;
        let (who, _) = name(&peer.id);
        // Two peers sharing a first segment show their whole ids.
        let label = if names.iter().filter(|n| **n == who).count() > 1 {
            peer.id.as_str()
        } else {
            who
        };
        let gutter = match (selected, focused) {
            (true, true) => Span::raw(glyphs.bar()).fg(ACCENT),
            (true, false) => Span::styled(glyphs.bar(), dim()),
            _ => Span::raw(" "),
        };
        let mut right = Vec::new();
        let unread = app.unread_with(&peer.id);
        if unread > 0 {
            right.push(Span::raw(format!("{}{unread}", glyphs.unread())).fg(ALERT));
        }
        let awaiting = app.awaiting_with(&peer.id);
        if awaiting > 0 {
            right.push(Span::styled(
                format!(" {}{awaiting}", glyphs.awaiting()),
                dim(),
            ));
        }
        let badges: usize = right.iter().map(Span::width).sum();
        let name_cols = cols.saturating_sub(5 + badges);
        let name = Span::raw(short(label, name_cols));
        let name = if selected { name.bold() } else { name };
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
}

fn draw_conversation(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
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
            None => vec![Line::styled("waking up…", dim())],
            Some(_) => vec![
                Line::from(glyphs.avatar("")),
                Line::default(),
                Line::from("it's quiet here"),
                Line::styled("trust someone with  deaddrop peer add", dim()),
            ],
        };
        frame.render_widget(Paragraph::new(heading("conversation", focused)), head);
        draw_empty(frame, body, lines);
        return;
    };

    let (who, rest) = name(&peer.id);
    frame.render_widget(
        Paragraph::new(spread(
            vec![
                Span::raw(format!("{} ", glyphs.avatar(&peer.id))),
                heading(who, focused),
                Span::styled(rest.to_owned(), dim()),
            ],
            vec![Span::styled("via relay", dim())],
            cols,
        )),
        head,
    );

    let thread = app.current_thread();
    if thread.is_empty() {
        draw_empty(
            frame,
            body,
            vec![
                Line::from(format!("nothing with {who} yet")),
                Line::styled("say hello with  deaddrop send", dim()),
            ],
        );
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selected = (0, 0);
    let mut last_author: Option<bool> = None;
    let mut last_era: Option<bool> = None;
    for (i, row) in thread.iter().enumerate() {
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
        if last_author != Some(row.outgoing()) {
            lines.push(Line::default());
            lines.push(if row.outgoing() {
                Line::from(Span::raw("  you").bold().fg(ACCENT))
            } else {
                Line::from(vec![
                    Span::raw(format!("  {} ", glyphs.avatar(row.peer()))),
                    Span::raw(who.to_owned()).bold(),
                ])
            });
            last_author = Some(row.outgoing());
        }
        let start = lines.len();
        lines.extend(message_lines(
            app,
            row,
            i == app.message,
            focused,
            cols,
            glyphs,
        ));
        if i == app.message {
            selected = (start, lines.len());
        }
    }

    let height = body.height as usize;
    let mut offset = lines.len().saturating_sub(height);
    if selected.0 < offset {
        offset = selected.0.saturating_sub(2);
    }
    if selected.1 > offset + height {
        offset = selected.1 - height;
    }
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), body);
}

fn centered_rule(label: &str, cols: usize, glyphs: Glyphs) -> Line<'static> {
    let side = cols.saturating_sub(width(label)) / 2;
    let rule = glyphs.rule().repeat(side.min(12));
    Line::styled(format!("{rule}{label}{rule}"), dim()).alignment(Alignment::Center)
}

fn message_lines(
    app: &App,
    row: &Row<'_>,
    selected: bool,
    focused: bool,
    cols: usize,
    glyphs: Glyphs,
) -> Vec<Line<'static>> {
    let mut meta: Vec<Span<'static>> = Vec::new();
    if row.kind() != "message" {
        meta.push(Span::styled(format!(" {}", row.kind()), dim()));
    }
    if !row.artifacts().is_empty() {
        meta.push(Span::styled(
            format!(" {}{}", glyphs.artifact(), row.artifacts().len()),
            dim(),
        ));
    }
    if let Row::Sent(m) = row {
        meta.push(if m.acked_by.is_empty() {
            Span::styled(format!(" {}", glyphs.awaiting()), dim())
        } else {
            Span::raw(format!(" {}", glyphs.check())).fg(GOOD)
        });
    }
    let meta_cols: usize = meta.iter().map(Span::width).sum();
    let gutter = if selected && focused {
        Span::raw(format!("{} ", glyphs.bar())).fg(ACCENT)
    } else if selected {
        Span::styled(format!("{} ", glyphs.bar()), dim())
    } else if app.is_unread(row.id()) {
        Span::raw(format!("{} ", glyphs.unread())).fg(ALERT)
    } else {
        Span::raw("  ")
    };

    let text_cols = cols.saturating_sub(4 + meta_cols);
    let body = if row.body().is_empty() {
        vec![String::new()]
    } else {
        wrap(row.body(), text_cols)
    };
    let last = body.len() - 1;
    body.into_iter()
        .enumerate()
        .map(|(n, text)| {
            let text = if row.body().is_empty() {
                Span::styled("(no body)", dim().add_modifier(Modifier::ITALIC))
            } else {
                Span::raw(text)
            };
            let left = vec![gutter.clone(), Span::raw("  "), text];
            if n == last {
                spread(left, meta.clone(), cols)
            } else {
                Line::from(left)
            }
        })
        .collect()
}

fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<8}"), dim()),
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
        .map(|l| Line::styled(format!("  {l}"), dim()))
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
                heading("vault", focused),
                Span::styled(" · message", dim()),
            ]));
            lines.push(Line::default());
            for (i, section) in Section::ALL.into_iter().enumerate() {
                let open = focused && i == app.section;
                let (marker, label) = if open {
                    (
                        Span::raw(format!("{} ", glyphs.expanded())).fg(ACCENT),
                        Span::raw(section.label()).bold(),
                    )
                } else {
                    (
                        Span::styled(format!("{} ", glyphs.collapsed()), dim()),
                        Span::raw(section.label()),
                    )
                };
                let summary = summary(app, row, section, glyphs);
                let room = cols.saturating_sub(3 + section.label().len());
                lines.push(spread(
                    vec![marker, label],
                    vec![Span::styled(short(&summary, room), dim())],
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
                heading("vault", focused),
                Span::styled(" · contact", dim()),
            ]));
            lines.push(Line::default());
            lines.push(Line::from("identity"));
            lines.extend(block(&peer.id, cols));
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::raw("trust  "),
                Span::styled("explicit · out of band", dim()),
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
            lines.push(Line::from(heading("vault", focused)));
            lines.push(Line::default());
            lines.push(Line::styled("nothing selected", dim()));
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
            None => vec![Line::styled("  no trusted key on file", dim())],
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
            vec![Line::styled("  no artifact refs", dim())]
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
                    lines.push(Line::styled("  no ack recorded yet", dim()));
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
            None => vec![Line::styled("  none", dim())],
        },
        Section::Raw => block(row.raw(), cols)
            .into_iter()
            .map(|l| l.style(dim()))
            .collect(),
    }
}

fn draw_compose(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let target = app
        .selected_peer()
        .map_or("someone".to_owned(), |p| name(&p.id).0.to_owned());
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(format!(" {} ", glyphs.prompt())).fg(ACCENT),
            Span::styled(
                format!("read-only for now · write to {target} with  deaddrop send"),
                dim().add_modifier(Modifier::ITALIC),
            ),
        ])),
        area,
    );
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let (up_down, right, left) = match glyphs {
        Glyphs::Unicode => ("↑↓", "→", "←"),
        Glyphs::Ascii => ("up/dn", "->", "<-"),
    };
    let keys: &[(&str, &str)] = match app.focus {
        Focus::Contacts => &[
            (up_down, "contacts"),
            (right, "open"),
            ("r", "sync"),
            ("q", "quit"),
        ],
        Focus::Conversation => &[
            (up_down, "messages"),
            (right, "vault"),
            (left, "contacts"),
            ("r", "sync"),
            ("q", "quit"),
        ],
        Focus::Vault => &[
            (up_down, "sections"),
            (left, "back"),
            ("tab", "contacts"),
            ("r", "sync"),
            ("q", "quit"),
        ],
    };
    let mut left_spans = vec![Span::raw(" ")];
    for (key, what) in keys {
        left_spans.push(Span::raw((*key).to_owned()).bold());
        left_spans.push(Span::styled(format!(" {what}   "), dim()));
    }
    let used: usize = left_spans.iter().map(Span::width).sum();
    let room = (area.width as usize).saturating_sub(used + 1);
    let status = Span::styled(format!("{} ", short(&app.status, room)), dim());
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
}
