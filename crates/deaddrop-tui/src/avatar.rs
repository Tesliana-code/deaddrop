//! Decorative glyphs. Nothing here is protocol state.

const FLOWERS: [&str; 6] = ["🌼", "🌻", "🥀", "🌷", "🌸", "🌺"];
const ASCII_FLOWERS: [&str; 6] = ["*", "@", "%", "$", "=", "^"];

/// Header activity: the [↓] mark turning. Plain ASCII, so both glyph sets.
const ARROWS: [&str; 4] = ["[^]", "[>]", "[v]", "[<]"];
/// Inline waiting: bracketed typing dots.
const TYPING: [&str; 5] = ["[·  ]", "[·· ]", "[···]", "[ ··]", "[  ·]"];
const ASCII_TYPING: [&str; 5] = ["[.  ]", "[.. ]", "[...]", "[ ..]", "[  .]"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    Unicode,
    Ascii,
}

/// FNV-1a: stable across runs and platforms, unlike `std`'s hasher.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

impl Glyphs {
    /// The same node id always gets the same avatar.
    pub fn avatar(self, node: &str) -> &'static str {
        let set = match self {
            Self::Unicode => &FLOWERS,
            Self::Ascii => &ASCII_FLOWERS,
        };
        set[(fnv1a(node) % set.len() as u64) as usize]
    }

    /// The header's activity spinner, a step every other frame.
    pub fn spinner(self, tick: usize) -> &'static str {
        ARROWS[(tick / 2) % ARROWS.len()]
    }

    /// The inline waiting indicator, a step every third frame: calm.
    pub fn typing(self, tick: usize) -> &'static str {
        let frames = match self {
            Self::Unicode => &TYPING,
            Self::Ascii => &ASCII_TYPING,
        };
        frames[(tick / 3) % frames.len()]
    }

    fn pick(self, unicode: &'static str, ascii: &'static str) -> &'static str {
        match self {
            Self::Unicode => unicode,
            Self::Ascii => ascii,
        }
    }

    pub fn idle(self) -> &'static str {
        self.pick("◌", "o")
    }

    pub fn inbound(self) -> &'static str {
        self.pick("←", "<")
    }

    pub fn outbound(self) -> &'static str {
        self.pick("→", ">")
    }

    pub fn check(self) -> &'static str {
        self.pick("✓", "+")
    }

    /// Sent, no ACK recorded yet.
    pub fn awaiting(self) -> &'static str {
        self.pick("◷", "~")
    }

    /// Arrived this session and not yet looked at. UI state only.
    pub fn unread(self) -> &'static str {
        self.pick("●", "#")
    }

    pub fn artifact(self) -> &'static str {
        self.pick("⧉", "&")
    }

    /// Relay reachable on the last refresh.
    pub fn relay_up(self) -> &'static str {
        self.pick("●", "o")
    }

    pub fn relay_down(self) -> &'static str {
        self.pick("○", "x")
    }

    /// Selection gutter.
    pub fn bar(self) -> &'static str {
        self.pick("▌", "|")
    }

    pub fn rule(self) -> &'static str {
        self.pick("─", "-")
    }

    pub fn divider(self) -> &'static str {
        self.pick("│", "|")
    }

    /// The product mark in the header: decoration, no meaning.
    pub fn mark(self) -> &'static str {
        self.pick("[↓]", "[v]")
    }

    pub fn prompt(self) -> &'static str {
        self.pick("›", ">")
    }

    pub fn collapsed(self) -> &'static str {
        self.pick("▸", ">")
    }

    pub fn expanded(self) -> &'static str {
        self.pick("▾", "v")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_is_stable_per_node() {
        for glyphs in [Glyphs::Unicode, Glyphs::Ascii] {
            let a = glyphs.avatar("node-a:shell:deaddrop");
            assert_eq!(a, glyphs.avatar("node-a:shell:deaddrop"));
        }
    }

    #[test]
    fn avatar_matches_across_glyph_sets() {
        let index = |g: Glyphs, set: &[&str; 6], node: &str| {
            set.iter().position(|f| *f == g.avatar(node)).unwrap()
        };
        for node in ["a:b:c", "danil:x:y", "tanish:x:y", "klodik:x:y"] {
            assert_eq!(
                index(Glyphs::Unicode, &FLOWERS, node),
                index(Glyphs::Ascii, &ASCII_FLOWERS, node)
            );
        }
    }

    #[test]
    fn avatars_vary_across_nodes() {
        let distinct: std::collections::BTreeSet<_> = (0..32)
            .map(|i| Glyphs::Unicode.avatar(&format!("node-{i}:shell:deaddrop")))
            .collect();
        assert_eq!(distinct.len(), FLOWERS.len());
    }

    #[test]
    fn spinner_cycles() {
        let header: Vec<&str> = (0..8)
            .step_by(2)
            .map(|t| Glyphs::Unicode.spinner(t))
            .collect();
        assert_eq!(header, ["[^]", "[>]", "[v]", "[<]"]);
        assert_eq!(Glyphs::Ascii.spinner(8), "[^]", "the same in ASCII");
        let dots: Vec<&str> = (0..15)
            .step_by(3)
            .map(|t| Glyphs::Unicode.typing(t))
            .collect();
        assert_eq!(dots, ["[·  ]", "[·· ]", "[···]", "[ ··]", "[  ·]"]);
        assert_eq!(Glyphs::Ascii.typing(6), "[...]");
        assert!((0..15).all(|t| Glyphs::Ascii.typing(t).is_ascii()));
        assert_eq!(Glyphs::Unicode.typing(15), "[·  ]", "it cycles");
    }
}
