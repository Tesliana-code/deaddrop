//! Terminal key events to app keys.
//!
//! Newlines in the draft: Ctrl+J is the one that works everywhere — a line
//! feed, which raw-mode terminals report as Ctrl+J. Shift+Enter inserts a
//! newline only when the terminal reports it apart from Enter (the kitty
//! keyboard protocol); most, Windows Terminal included, send a bare Enter for
//! both, so it sends. Alt+Enter is taken too where it arrives, but Windows
//! Terminal keeps it for full screen. The footer advertises only Ctrl+J.
//!
//! Observed in Windows Terminal → WSL (crossterm 0.28, probe, 2026-10-03):
//! Enter and Shift+Enter both `Enter` with no modifiers; Ctrl+J
//! `Char('j')` + CONTROL; Alt+Enter no event; keyboard enhancement not
//! supported.

use std::io::Write;

use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseButton, MouseEvent, MouseEventKind,
};

#[cfg(test)]
use crate::app::{ComposerView, ContactsView, ConversationView};
use crate::app::{Drawn, Key};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Key(Key),
    /// Quit the app.
    Quit,
}

pub fn translate(event: KeyEvent) -> Option<Input> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    // Control chords are not text; keep them out of the draft.
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        return match event.code {
            // Copies a selection, else quits (the app decides).
            KeyCode::Char('c') => Some(Input::Key(Key::Copy)),
            KeyCode::Char('j') => Some(Input::Key(Key::Newline)),
            // Observed distinct in Windows Terminal → WSL: the keyboard
            // fallback for scrolling.
            KeyCode::Up => Some(Input::Key(Key::ScrollUp)),
            KeyCode::Down => Some(Input::Key(Key::ScrollDown)),
            _ => None,
        };
    }
    // Alt+Up / Alt+Down are reserved: reported distinctly, assigned later.
    if event.modifiers.contains(KeyModifiers::ALT)
        && matches!(event.code, KeyCode::Up | KeyCode::Down)
    {
        return None;
    }
    let key = match event.code {
        KeyCode::Enter
            if event
                .modifiers
                .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
        {
            Key::Newline
        }
        KeyCode::Enter => Key::Enter,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Esc => Key::Esc,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::BackTab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        // Optional where the keyboard has them.
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    };
    Some(Input::Key(key))
}

/// Mouse input by pane. Over the conversation: the wheel scrolls, and the
/// left button selects text (a plain click focuses). Over the contacts: the
/// wheel moves the selection, a click opens. Drags and releases are passed
/// on wherever they happen, so a selection can be finished outside the
/// pane. Other buttons and plain moves are ignored.
pub fn translate_mouse(event: MouseEvent, drawn: &Drawn) -> Option<Key> {
    let (column, row) = (event.column, event.row);
    let over = drawn
        .conversation
        .as_ref()
        .is_some_and(|v| v.contains(column, row));
    let contact = drawn.contacts.as_ref().and_then(|c| c.at(column, row));
    let over_contacts = drawn
        .contacts
        .as_ref()
        .is_some_and(|c| (c.x..c.x.saturating_add(c.width)).contains(&column));
    match event.kind {
        MouseEventKind::ScrollUp if over => Some(Key::WheelUp),
        MouseEventKind::ScrollDown if over => Some(Key::WheelDown),
        MouseEventKind::ScrollUp if over_contacts => Some(Key::ContactsUp),
        MouseEventKind::ScrollDown if over_contacts => Some(Key::ContactsDown),
        MouseEventKind::Down(MouseButton::Left)
            if drawn.wire_toggle.is_some_and(|l| l.contains(column, row)) =>
        {
            Some(Key::ToggleWire)
        }
        MouseEventKind::Down(MouseButton::Left)
            if drawn
                .contacts
                .as_ref()
                .and_then(|c| c.room_at(column, row))
                .is_some() =>
        {
            drawn
                .contacts
                .as_ref()
                .and_then(|c| c.room_at(column, row))
                .map(Key::ClickRoom)
        }
        MouseEventKind::Down(MouseButton::Left) => match contact {
            Some(i) => Some(Key::ClickContact(i)),
            None if over => Some(Key::MouseDown { column, row }),
            None if drawn
                .composer
                .as_ref()
                .is_some_and(|c| c.contains(column, row)) =>
            {
                Some(Key::ComposerClick { column, row })
            }
            None => None,
        },
        // Right click pastes, in the composer only.
        MouseEventKind::Down(MouseButton::Right)
            if drawn
                .composer
                .as_ref()
                .is_some_and(|c| c.contains(column, row)) =>
        {
            Some(Key::ComposerRightClick)
        }
        MouseEventKind::Drag(MouseButton::Left) => Some(Key::MouseDrag { column, row }),
        MouseEventKind::Up(MouseButton::Left) => Some(Key::MouseUp { column, row }),
        _ => None,
    }
}

/// An OSC 52 sequence asking the terminal to put `text` on the clipboard.
/// Observed to work in Windows Terminal → WSL, Unicode and emoji intact.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Hand the mouse and paste handling back to the terminal. Run on every
/// exit, panics included.
pub fn release_mouse(out: &mut impl Write) -> std::io::Result<()> {
    ratatui::crossterm::execute!(out, DisableBracketedPaste, DisableMouseCapture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::KeyEventState;

    fn press(code: KeyCode, modifiers: KeyModifiers) -> Option<Input> {
        translate(KeyEvent::new(code, modifiers))
    }

    // The events below are what crossterm reports in raw mode, as observed
    // with a key-event probe in Windows Terminal → WSL and in tmux.

    #[test]
    fn enter_sends() {
        assert_eq!(
            press(KeyCode::Enter, KeyModifiers::NONE),
            Some(Input::Key(Key::Enter))
        );
    }

    #[test]
    fn ctrl_j_is_a_newline() {
        assert_eq!(
            press(KeyCode::Char('j'), KeyModifiers::CONTROL),
            Some(Input::Key(Key::Newline))
        );
        assert_eq!(
            press(KeyCode::Char('j'), KeyModifiers::NONE),
            Some(Input::Key(Key::Char('j'))),
            "plain j is text"
        );
    }

    #[test]
    fn shift_enter_is_a_newline_only_when_reported_apart() {
        assert_eq!(
            press(KeyCode::Enter, KeyModifiers::SHIFT),
            Some(Input::Key(Key::Newline))
        );
        // Where the terminal folds Shift+Enter into Enter, this is all that
        // arrives, and it sends.
        assert_eq!(
            press(KeyCode::Enter, KeyModifiers::NONE),
            Some(Input::Key(Key::Enter))
        );
    }

    #[test]
    fn alt_enter_is_a_newline_where_it_arrives() {
        assert_eq!(
            press(KeyCode::Enter, KeyModifiers::ALT),
            Some(Input::Key(Key::Newline))
        );
    }

    #[test]
    fn ctrl_c_quits_and_other_chords_are_dropped() {
        assert_eq!(
            press(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Input::Key(Key::Copy)),
            "copies a selection, else quits"
        );
        assert_eq!(press(KeyCode::Char('x'), KeyModifiers::CONTROL), None);
    }

    #[test]
    fn releases_are_ignored() {
        let mut event = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        event.kind = KeyEventKind::Release;
        event.state = KeyEventState::NONE;
        assert_eq!(translate(event), None);
    }

    #[test]
    fn unicode_and_shifted_letters_are_text() {
        assert_eq!(
            press(KeyCode::Char('Ж'), KeyModifiers::SHIFT),
            Some(Input::Key(Key::Char('Ж')))
        );
        assert_eq!(
            press(KeyCode::Char('🌼'), KeyModifiers::NONE),
            Some(Input::Key(Key::Char('🌼')))
        );
    }

    #[test]
    fn ctrl_arrows_scroll_and_alt_arrows_are_reserved() {
        assert_eq!(
            press(KeyCode::Up, KeyModifiers::CONTROL),
            Some(Input::Key(Key::ScrollUp))
        );
        assert_eq!(
            press(KeyCode::Down, KeyModifiers::CONTROL),
            Some(Input::Key(Key::ScrollDown))
        );
        assert_eq!(press(KeyCode::Up, KeyModifiers::ALT), None);
        assert_eq!(press(KeyCode::Down, KeyModifiers::ALT), None);
        assert_eq!(
            press(KeyCode::Up, KeyModifiers::NONE),
            Some(Input::Key(Key::Up)),
            "plain arrows are untouched"
        );
    }

    #[test]
    fn page_keys_map_where_they_exist() {
        for (code, key) in [
            (KeyCode::PageUp, Key::PageUp),
            (KeyCode::PageDown, Key::PageDown),
            (KeyCode::Home, Key::Home),
            (KeyCode::End, Key::End),
            (KeyCode::Delete, Key::Delete),
        ] {
            assert_eq!(press(code, KeyModifiers::NONE), Some(Input::Key(key)));
        }
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn drawn() -> Drawn {
        Drawn {
            conversation: Some(ConversationView {
                x: 30,
                y: 4,
                width: 60,
                height: 15,
                ..ConversationView::default()
            }),
            contacts: Some(ContactsView {
                x: 1,
                y: 4,
                width: 24,
                rows: 3,
                rooms_y: 0,
                room_rows: 0,
            }),
            composer: Some(ComposerView {
                x: 0,
                y: 21,
                width: 100,
                height: 2,
                text_x: 3,
                cols: 96,
                offset: 0,
            }),
            wire_toggle: None,
        }
    }

    #[test]
    fn the_wheel_scrolls_only_over_the_conversation() {
        let d = drawn();
        let t = |kind, c, r| translate_mouse(mouse(kind, c, r), &d);
        assert_eq!(t(MouseEventKind::ScrollUp, 40, 10), Some(Key::WheelUp));
        assert_eq!(t(MouseEventKind::ScrollDown, 30, 4), Some(Key::WheelDown));
        assert_eq!(t(MouseEventKind::ScrollUp, 100, 10), None, "vault");
        assert_eq!(t(MouseEventKind::ScrollUp, 40, 19), None, "composer");
        assert_eq!(
            translate_mouse(mouse(MouseEventKind::ScrollUp, 40, 10), &Drawn::default()),
            None
        );
        for kind in [
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Moved,
        ] {
            assert_eq!(t(kind, 40, 10), None, "{kind:?}");
            assert_eq!(t(kind, 5, 5), None, "{kind:?}");
        }
        // Wheel over the contact column moves the contact selection.
        assert_eq!(t(MouseEventKind::ScrollUp, 10, 10), Some(Key::ContactsUp));
        assert_eq!(
            t(MouseEventKind::ScrollDown, 10, 5),
            Some(Key::ContactsDown)
        );
        // Drags and releases go through wherever they are.
        assert_eq!(
            t(MouseEventKind::Drag(MouseButton::Left), 5, 5),
            Some(Key::MouseDrag { column: 5, row: 5 })
        );
        assert_eq!(
            t(MouseEventKind::Up(MouseButton::Left), 40, 10),
            Some(Key::MouseUp {
                column: 40,
                row: 10
            })
        );
    }

    #[test]
    fn a_left_click_opens_a_contact_or_focuses_the_conversation() {
        let d = drawn();
        let click =
            |c, r| translate_mouse(mouse(MouseEventKind::Down(MouseButton::Left), c, r), &d);
        assert_eq!(click(5, 4), Some(Key::ClickContact(0)));
        assert_eq!(click(20, 6), Some(Key::ClickContact(2)));
        assert_eq!(click(5, 7), None, "below the last contact");
        assert_eq!(click(5, 3), None, "the heading");
        assert_eq!(
            click(40, 10),
            Some(Key::MouseDown {
                column: 40,
                row: 10
            })
        );
        assert_eq!(click(40, 25), None, "outside everything");
        assert_eq!(
            click(10, 22),
            Some(Key::ComposerClick {
                column: 10,
                row: 22
            }),
            "the composer is clickable"
        );
        // Right click: Windows Terminal sends only the mouse event under
        // capture, so the composer pastes from the clipboard itself.
        assert_eq!(
            translate_mouse(mouse(MouseEventKind::Down(MouseButton::Right), 10, 22), &d),
            Some(Key::ComposerRightClick)
        );
        for (c, r) in [(40, 10), (5, 5), (40, 25)] {
            assert_eq!(
                translate_mouse(mouse(MouseEventKind::Down(MouseButton::Right), c, r), &d),
                None,
                "right click at {c},{r} outside the composer reads nothing"
            );
        }
    }

    #[test]
    fn releasing_the_mouse_disables_every_capture_mode() {
        let mut out = Vec::new();
        release_mouse(&mut out).unwrap();
        let written = String::from_utf8(out).unwrap();
        for mode in ["?1000l", "?1002l", "?1003l", "?1006l", "?2004l"] {
            assert!(written.contains(mode), "{mode} in {written:?}");
        }
    }

    #[test]
    fn osc52_encodes_utf8_exactly() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        // The string proven in Windows Terminal, encoded as `base64 -w0` did.
        assert_eq!(
            osc52("tri srca šđčćž 🐙 OK-52"),
            "\x1b]52;c;dHJpIHNyY2EgxaHEkcSNxIfFviDwn5CZIE9LLTUy\x07"
        );
    }
}
