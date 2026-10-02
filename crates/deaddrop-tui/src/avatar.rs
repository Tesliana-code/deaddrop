//! Decorative glyphs. Nothing here is protocol state.

const FLOWERS: [&str; 3] = ["🌼", "🌻", "🥀"];
const ASCII_FLOWERS: [&str; 3] = ["*", "@", "%"];

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

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

    pub fn spinner(self, tick: usize) -> &'static str {
        match self {
            Self::Unicode => SPINNER[tick % SPINNER.len()],
            Self::Ascii => ASCII_SPINNER[tick % ASCII_SPINNER.len()],
        }
    }

    pub fn idle(self) -> &'static str {
        match self {
            Self::Unicode => "◌",
            Self::Ascii => "o",
        }
    }

    pub fn inbound(self) -> &'static str {
        match self {
            Self::Unicode => "←",
            Self::Ascii => "<",
        }
    }

    pub fn outbound(self) -> &'static str {
        match self {
            Self::Unicode => "→",
            Self::Ascii => ">",
        }
    }

    pub fn check(self) -> &'static str {
        match self {
            Self::Unicode => "✓",
            Self::Ascii => "+",
        }
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
        let index = |g: Glyphs, set: &[&str; 3], node: &str| {
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
        assert_eq!(Glyphs::Unicode.spinner(0), Glyphs::Unicode.spinner(10));
        assert_ne!(Glyphs::Unicode.spinner(0), Glyphs::Unicode.spinner(1));
        assert_eq!(Glyphs::Ascii.spinner(1), "/");
    }
}
