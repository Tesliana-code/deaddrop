//! The V0 capability registry and the resolver.
//!
//! A capability says what a peer can do. It is never permission: a worker
//! is selected only if the capability exists exactly, the peer is trusted
//! (a channel), reachable in the room, allowed by local policy (permission
//! to ask), and the capability's mode fits V0's authority (read-only or
//! report-only; nothing here can change the world).

/// Public web lookup by the Research peer (WebSearch only).
pub const WEB_SEARCH: &str = "web.search";
/// The GitHub peer's one read-only question.
pub const GITHUB_INSPECT: &str = deaddrop_github::CAPABILITY;
/// Klodik combining reports it is given into one answer.
pub const SYNTHESIZE: &str = "synthesize";

/// What a capability may do. There is deliberately no variant that writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Reads public or repository state; changes nothing.
    ReadOnly,
    /// Reasons over reports it is handed; reaches nothing outside.
    ReportOnly,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::ReportOnly => "report-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capability {
    pub id: &'static str,
    pub peer: &'static str,
    pub mode: Mode,
}

/// Exactly the three real workers. No aliases, no fuzzy names.
pub const REGISTRY: [Capability; 3] = [
    Capability {
        id: WEB_SEARCH,
        peer: "research:agent:deaddrop",
        mode: Mode::ReadOnly,
    },
    Capability {
        id: GITHUB_INSPECT,
        peer: "github:agent:deaddrop",
        mode: Mode::ReadOnly,
    },
    Capability {
        id: SYNTHESIZE,
        peer: "klodik:agent:deaddrop",
        mode: Mode::ReportOnly,
    },
];

/// Exact lookup: `Web.Search` or `search` is not `web.search`.
pub fn capability(id: &str) -> Option<&'static Capability> {
    REGISTRY.iter().find(|c| c.id == id)
}

/// What this node knows when resolving: its trusted peers (channels), the
/// room's members (reachability) and its own task policy (who it may ask).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    pub trusted: Vec<String>,
    pub members: Vec<String>,
    pub may_ask: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    UnknownCapability(String),
    Untrusted { capability: String, peer: String },
    NotInRoom { capability: String, peer: String },
    PolicyDenied { capability: String, peer: String },
    Authority { capability: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let short = |p: &str| p.split(':').next().unwrap_or(p).to_owned();
        match self {
            Self::UnknownCapability(c) => write!(f, "unknown capability {c:?}"),
            Self::Untrusted { capability, peer } => {
                write!(f, "{capability}: {} is not a trusted peer", short(peer))
            }
            Self::NotInRoom { capability, peer } => {
                write!(f, "{capability}: {} is not in the room", short(peer))
            }
            Self::PolicyDenied { capability, peer } => {
                write!(
                    f,
                    "{capability}: policy does not allow asking {}",
                    short(peer)
                )
            }
            Self::Authority { capability } => {
                write!(f, "{capability}: needs authority V0 does not grant")
            }
        }
    }
}

/// The worker for `id`, if every condition holds; otherwise the first one
/// that does not, in the order trust, reachability, policy, authority.
pub fn resolve(id: &str, ctx: &Context) -> Result<&'static Capability, Refusal> {
    let c = capability(id).ok_or_else(|| Refusal::UnknownCapability(id.to_owned()))?;
    let (capability, peer) = (c.id.to_owned(), c.peer.to_owned());
    let has = |list: &[String]| list.iter().any(|p| p == c.peer);
    if !has(&ctx.trusted) {
        return Err(Refusal::Untrusted { capability, peer });
    }
    if !has(&ctx.members) {
        return Err(Refusal::NotInRoom { capability, peer });
    }
    if !has(&ctx.may_ask) {
        return Err(Refusal::PolicyDenied { capability, peer });
    }
    if !matches!(c.mode, Mode::ReadOnly | Mode::ReportOnly) {
        return Err(Refusal::Authority { capability });
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Context {
        let peers: Vec<String> = REGISTRY.iter().map(|c| c.peer.to_owned()).collect();
        Context {
            trusted: peers.clone(),
            members: peers.clone(),
            may_ask: peers,
        }
    }

    #[test]
    fn exactly_three_real_capabilities_no_aliases() {
        let ids: Vec<&str> = REGISTRY.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["web.search", "github.inspect", "synthesize"]);
        assert_eq!(
            GITHUB_INSPECT,
            deaddrop_github::CAPABILITY,
            "the worker's own id"
        );
        for alias in [
            "search",
            "Web.Search",
            "web.search ",
            "github",
            "synthesis",
            "runpod",
        ] {
            assert_eq!(capability(alias), None, "{alias}");
        }
    }

    #[test]
    fn capable_and_allowed_resolves_to_its_peer() {
        assert_eq!(
            resolve("web.search", &all()).unwrap().peer,
            "research:agent:deaddrop"
        );
        assert_eq!(
            resolve("github.inspect", &all()).unwrap().peer,
            "github:agent:deaddrop"
        );
        assert_eq!(
            resolve("synthesize", &all()).unwrap().peer,
            "klodik:agent:deaddrop"
        );
        assert_eq!(
            resolve("deploy", &all()),
            Err(Refusal::UnknownCapability("deploy".into()))
        );
    }

    #[test]
    fn capability_is_not_permission() {
        let research = "research:agent:deaddrop".to_owned();
        let without = |list: &mut Vec<String>| list.retain(|p| *p != research);
        let mut ctx = all();
        without(&mut ctx.may_ask);
        assert_eq!(
            resolve("web.search", &ctx).unwrap_err().to_string(),
            "web.search: policy does not allow asking research"
        );
        let mut ctx = all();
        without(&mut ctx.trusted);
        assert!(matches!(
            resolve("web.search", &ctx),
            Err(Refusal::Untrusted { .. })
        ));
        let mut ctx = all();
        without(&mut ctx.members);
        assert!(matches!(
            resolve("web.search", &ctx),
            Err(Refusal::NotInRoom { .. })
        ));
        // Policy on one peer says nothing about another.
        assert!(resolve("github.inspect", &ctx).is_ok());
    }

    #[test]
    fn no_registered_capability_writes() {
        assert!(
            REGISTRY
                .iter()
                .all(|c| matches!(c.mode, Mode::ReadOnly | Mode::ReportOnly))
        );
    }
}
