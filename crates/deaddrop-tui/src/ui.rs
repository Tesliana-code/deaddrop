//! Rendering. Reads [`App`] and draws it; never touches the network.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::{Activity, App, Focus, Row, View};
use crate::avatar::Glyphs;
use crate::snapshot::{Peer, Received, Sent};

/// Shorten an identifier to `max` characters, marking the cut.
pub fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// The first line of a body, for one-line previews.
fn preview(body: &str) -> &str {
    body.lines().next().unwrap_or_default()
}

pub fn draw(frame: &mut Frame, app: &App, glyphs: Glyphs, tick: usize) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(5),
        Constraint::Length(4),
    ])
    .areas(frame.area());

    draw_header(frame, header, app, glyphs);
    match app.view {
        View::List => draw_lists(frame, body, app, glyphs),
        View::Detail => draw_detail(frame, body, app, glyphs),
    }
    draw_footer(frame, footer, app, glyphs, tick);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let block = Block::bordered().title(" DEADDROP ".bold());
    let lines = match &app.snapshot {
        None => vec![Line::from("loading node…".dim())],
        Some(s) => vec![
            Line::from(vec![
                Span::raw(format!("{} ", glyphs.avatar(&s.node.id))),
                Span::raw(s.node.id.clone()).bold(),
                Span::raw("  relay ").dim(),
                Span::raw(s.node.relay.clone()),
            ]),
            Line::from(
                format!(
                    "{} peers · {} received · {} sent · {} acked",
                    s.peers.len(),
                    s.inbox.len(),
                    s.sent.len(),
                    s.sent.iter().filter(|m| !m.acked_by.is_empty()).count()
                )
                .dim(),
            ),
        ],
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn pane(title: &str, focused: bool) -> Block<'_> {
    let block = Block::bordered().title(format!(" {title} "));
    if focused {
        block.border_style(Style::new().fg(Color::Yellow))
    } else {
        block.border_style(Style::new().dim())
    }
}

fn highlighted(list: List<'_>) -> List<'_> {
    list.highlight_style(Style::new().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▸ ")
}

fn draw_lists(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).areas(area);

    let peers = pane("PEERS", app.focus == Focus::Peers);
    if app.peers().is_empty() {
        frame.render_widget(
            Paragraph::new("no trusted peers yet\n\nadd one with\n`deaddrop peer add`".dim())
                .wrap(Wrap { trim: false })
                .block(peers),
            left,
        );
    } else {
        let items: Vec<ListItem> = app
            .peers()
            .iter()
            .map(|p| ListItem::new(peer_line(p, glyphs)))
            .collect();
        let mut state = ListState::default().with_selected(Some(app.peer));
        frame.render_stateful_widget(highlighted(List::new(items).block(peers)), left, &mut state);
    }

    let messages = pane("MESSAGES", app.focus == Focus::Messages);
    let rows = app.messages();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("inbox is empty — press r to sync".dim()).block(messages),
            right,
        );
    } else {
        let items: Vec<ListItem> = rows
            .iter()
            .map(|r| ListItem::new(message_line(r, glyphs)))
            .collect();
        let mut state = ListState::default().with_selected(Some(app.message));
        frame.render_stateful_widget(
            highlighted(List::new(items).block(messages)),
            right,
            &mut state,
        );
    }
}

fn peer_line(peer: &Peer, glyphs: Glyphs) -> Line<'static> {
    Line::from(format!("{} {}", glyphs.avatar(&peer.id), peer.id))
}

fn message_line(row: &Row<'_>, glyphs: Glyphs) -> Line<'static> {
    match row {
        Row::Received(m) => Line::from(vec![
            Span::raw(format!("{} ", glyphs.inbound())).fg(Color::Cyan),
            Span::raw(format!("{} ", glyphs.avatar(&m.from))),
            Span::raw(short(&m.from, 18)).bold(),
            Span::raw(format!("  {}  ", short(&m.id, 10))).dim(),
            Span::raw(preview(&m.body).to_owned()),
        ]),
        Row::Sent(m) => {
            let mut spans = vec![
                Span::raw(format!("{} ", glyphs.outbound())).fg(Color::Magenta),
                Span::raw(format!("{} ", glyphs.avatar(&m.to))),
                Span::raw(short(&m.to, 18)).bold(),
                Span::raw(format!("  {}  ", short(&m.id, 10))).dim(),
                Span::raw(preview(&m.body).to_owned()),
            ];
            if !m.acked_by.is_empty() {
                spans.push(Span::raw(format!("  {} ACK", glyphs.check())).fg(Color::Green));
            }
            Line::from(spans)
        }
    }
}

fn field(name: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{name:>12}  ")).dim(),
        Span::raw(value.into()),
    ])
}

fn draw_detail(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs) {
    let (title, lines) = match app.focus {
        Focus::Peers => match app.selected_peer() {
            Some(peer) => (
                format!("{} {}", glyphs.avatar(&peer.id), peer.id),
                peer_detail(app, peer, glyphs),
            ),
            None => ("PEER".into(), vec![]),
        },
        Focus::Messages => match app.selected_message() {
            Some(Row::Received(m)) => ("MESSAGE".into(), received_detail(m)),
            Some(Row::Sent(m)) => ("SENT".into(), sent_detail(m)),
            None => ("MESSAGE".into(), vec![]),
        },
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(pane(&title, true)),
        area,
    );
}

fn peer_detail(app: &App, peer: &Peer, glyphs: Glyphs) -> Vec<Line<'static>> {
    let mut lines = vec![
        field("node", peer.id.clone()),
        field("key", peer.key.clone()),
        field("trust", "explicit (peers.json)"),
        Line::default(),
    ];
    let rows = app.messages_with(&peer.id);
    if rows.is_empty() {
        lines.push(Line::from("no messages with this peer yet".dim()));
    }
    lines.extend(rows.iter().map(|r| message_line(r, glyphs)));
    lines
}

fn received_detail(m: &Received) -> Vec<Line<'static>> {
    let mut lines = vec![
        field("id", m.id.clone()),
        field("from", m.from.clone()),
        field("kind", m.kind.clone()),
    ];
    if let Some(c) = &m.correlation {
        lines.push(field("correlation", c.clone()));
    }
    for a in &m.artifacts {
        lines.push(field("artifact", a.clone()));
    }
    lines.push(field("delivery", m.delivery.join(" → ")));
    lines.push(Line::default());
    lines.extend(m.body.lines().map(|l| Line::from(l.to_owned())));
    lines
}

fn sent_detail(m: &Sent) -> Vec<Line<'static>> {
    let mut lines = vec![
        field("id", m.id.clone()),
        field("to", m.to.clone()),
        field("kind", m.kind.clone()),
    ];
    if let Some(c) = &m.correlation {
        lines.push(field("correlation", c.clone()));
    }
    for a in &m.artifacts {
        lines.push(field("artifact", a.clone()));
    }
    lines.push(field(
        "ack",
        if m.acked_by.is_empty() {
            "not yet acknowledged".to_owned()
        } else {
            format!(
                "acknowledged by {} (receipt only, not task success)",
                m.acked_by.join(", ")
            )
        },
    ));
    lines.push(Line::default());
    lines.extend(m.body.lines().map(|l| Line::from(l.to_owned())));
    lines
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, glyphs: Glyphs, tick: usize) {
    let activity = match app.activity {
        Activity::Busy(label) => Line::from(vec![
            Span::raw(format!("{} ", glyphs.spinner(tick))).fg(Color::Yellow),
            Span::raw(format!("{label}…")),
        ]),
        Activity::Idle => Line::from(vec![
            Span::raw(format!("{} ", glyphs.idle())).dim(),
            Span::raw(app.status.clone()).dim(),
        ]),
    };
    let help = match app.view {
        View::List => "↑↓ select  Tab pane  Enter open  r refresh  q/Esc quit",
        View::Detail => "Esc back  r refresh  q quit",
    };
    frame.render_widget(
        Paragraph::new(vec![activity, Line::from(help.dim())]).block(Block::bordered()),
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
    fn preview_is_first_line() {
        assert_eq!(preview("hello\nworld"), "hello");
        assert_eq!(preview(""), "");
    }
}
