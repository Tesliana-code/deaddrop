//! Deaddrop's own colors. The app paints its whole viewport with these, so
//! it looks the same whatever theme the host terminal uses.
//!
//! Font boundary: the TUI does not, and cannot, choose the font. The host
//! terminal owns it, along with size, line height and rendering; the app sends
//! no font-changing escapes and touches no terminal settings. The design
//! target is a clean monospace in the Droid Sans Mono style. A future Deaddrop
//! Desktop Shell may own the exact font, size, line height, DPI and rendering;
//! before any font is bundled there, its redistribution license must be
//! verified.
//!
//! Color carries meaning only where it says something — who wrote a message,
//! whether an ACK is recorded, whether something failed. Bodies stay plain.
//! Avatars are decoration and get no semantic color.

use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    /// The whole viewport.
    pub background: Color,
    /// Kept for places where a subtle separation earns its place.
    pub surface: Color,
    /// Body text. Never pure white.
    pub text: Color,
    /// Metadata, ids, secondary status, awaiting ACK.
    pub muted: Color,
    /// You: outgoing author and selection, the compose prompt.
    pub you: Color,
    /// Them: incoming author and selection, unread.
    pub peer: Color,
    /// The vault's own cues, and nothing else.
    pub vault: Color,
    /// A recorded, verified ACK; a reachable relay.
    pub ack: Color,
    /// Real failures only.
    pub error: Color,
    /// Rules and pane separators.
    pub divider: Color,
    /// Kept restrained; never a large block.
    pub selection_surface: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const NIGHT_GARDEN: Theme = Theme {
    name: "DEADDROP // NIGHT GARDEN",
    background: rgb(0x090C12),
    surface: rgb(0x10151E),
    text: rgb(0xD8DCE5),
    muted: rgb(0x737A88),
    you: rgb(0x63C7F5),
    peer: rgb(0xE8BC55),
    vault: rgb(0xA995E8),
    ack: rgb(0x72D68A),
    error: rgb(0xE06C75),
    divider: rgb(0x454B55),
    selection_surface: rgb(0x253246),
};

/// Author colors, IRC style: one per peer, picked from its authenticated
/// node id. Color means identity only — never trust, authority, success or
/// failure — so there is no red, no green and no `you` cyan here.
pub const AUTHORS: [Color; 6] = [
    rgb(0xE8BC55), // amber
    rgb(0xF0956A), // peach
    rgb(0xD7C9A6), // sand
    rgb(0xC3A6F7), // lilac
    rgb(0xE28AD0), // orchid
    rgb(0x8C9EFF), // periwinkle
];

/// FNV-1a: fixed, so a peer keeps its color across sessions and builds.
fn fnv1a(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c_9dc5, |h, b| {
        (h ^ u32::from(*b)).wrapping_mul(0x0100_0193)
    })
}

impl Theme {
    /// A peer's author color, from its node id. Same id, same color.
    pub fn author(&self, node: &str) -> Color {
        AUTHORS[fnv1a(node.as_bytes()) as usize % AUTHORS.len()]
    }

    /// Every cell starts here: readable text on the app's own background.
    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.text)
    }

    pub fn muted(&self) -> Style {
        Style::new().fg(self.muted)
    }

    pub fn divider(&self) -> Style {
        Style::new().fg(self.divider)
    }

    pub fn error(&self) -> Style {
        Style::new().fg(self.error)
    }

    pub fn ack(&self) -> Style {
        Style::new().fg(self.ack)
    }

    /// The color of whoever wrote a message.
    pub fn side(&self, outgoing: bool) -> Color {
        if outgoing { self.you } else { self.peer }
    }

    /// An author or heading in `color`.
    pub fn strong(&self, color: Color) -> Style {
        Style::new().fg(color).add_modifier(Modifier::BOLD)
    }

    /// The status line: muted unless it reports a real failure.
    pub fn status(&self, status: &str) -> Style {
        const FAILURES: [&str; 3] = ["send failed", "refresh failed", "relay unreachable"];
        if FAILURES.iter().any(|f| status.starts_with(f)) {
            self.error()
        } else {
            self.muted()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Theme = NIGHT_GARDEN;

    #[test]
    fn night_garden_is_the_specified_palette() {
        assert_eq!(T.background, Color::Rgb(0x09, 0x0C, 0x12));
        assert_eq!(T.text, Color::Rgb(0xD8, 0xDC, 0xE5));
        assert_eq!(T.you, Color::Rgb(0x63, 0xC7, 0xF5));
        assert_eq!(T.peer, Color::Rgb(0xE8, 0xBC, 0x55));
        assert_eq!(T.vault, Color::Rgb(0xA9, 0x95, 0xE8));
        assert_eq!(T.ack, Color::Rgb(0x72, 0xD6, 0x8A));
        assert_eq!(T.error, Color::Rgb(0xE0, 0x6C, 0x75));
        assert_eq!(T.divider, Color::Rgb(0x45, 0x4B, 0x55));
        assert_ne!(T.text, Color::Rgb(0xFF, 0xFF, 0xFF), "never pure white");
    }

    #[test]
    fn base_paints_its_own_background() {
        assert_eq!(T.base().bg, Some(T.background));
        assert_eq!(T.base().fg, Some(T.text));
    }

    #[test]
    fn direction_picks_the_author_color() {
        assert_eq!(T.side(true), T.you);
        assert_eq!(T.side(false), T.peer);
        assert_ne!(T.you, T.peer);
    }

    fn luminance(c: Color) -> f64 {
        let Color::Rgb(r, g, b) = c else { panic!() };
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
    }

    #[test]
    fn author_colors_are_stable_distinct_and_mean_identity_only() {
        let ids = [
            "klodik:agent:deaddrop",
            "github:agent:deaddrop",
            "research:agent:deaddrop",
        ];
        let colors: Vec<Color> = ids.iter().map(|id| T.author(id)).collect();
        // Pinned: the same peer is the same color in every session and build.
        assert_eq!(colors, [AUTHORS[5], AUTHORS[0], AUTHORS[4]]);
        assert_eq!(T.author(ids[0]), T.author(ids[0]));
        for (i, a) in colors.iter().enumerate() {
            assert!(
                colors[i + 1..].iter().all(|b| b != a),
                "agents stay distinct"
            );
        }
        let bg = luminance(T.background);
        for c in AUTHORS {
            // Not the colors that mean you, success, failure or the vault.
            assert!(![T.you, T.ack, T.error, T.vault].contains(&c));
            let contrast = (luminance(c) + 0.05) / (bg + 0.05);
            assert!(contrast >= 7.0, "{c:?} contrast {contrast:.1}");
        }
    }

    #[test]
    fn only_failures_are_red() {
        for failure in [
            "send failed: relay down",
            "refresh failed: locked",
            "relay unreachable · showing local state · x",
        ] {
            assert_eq!(T.status(failure), T.error(), "{failure}");
        }
        for quiet in [
            "sent to danil",
            "synced · 2 new · 1 ack · 0 rejected",
            "waking up",
        ] {
            assert_eq!(T.status(quiet), T.muted(), "{quiet}");
        }
    }
}
