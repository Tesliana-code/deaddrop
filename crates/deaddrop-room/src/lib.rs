//! Rooms over ordinary Deaddrop messages.
//!
//! A room message is application content inside a normal signed message
//! body; EnvelopeV0 is unchanged. One logical room message reaches each
//! member as its own signed delivery, all sharing one room message id, so
//! readers can show it once. The author is the envelope's verified sender,
//! never a header field.
//!
//! ```text
//! deaddrop-room/0
//! room: d34ddr0p
//! id: <room message id>
//! kind: message | request | report
//! hop: 0
//! mentions: research:agent:deaddrop        (requests only)
//! reply-to: <room message id>               (reports only)
//! status: ok | refused | failed             (reports only)
//!
//! <text>
//! ```
//!
//! Kinds: a `message` informs and invokes nobody; a `request` asks the peers
//! it mentions; a `report` answers a request and invokes nobody. Being in a
//! room grants neither invocation nor authority: whether a request is acted
//! on is the receiving peer's own policy.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const ROOM_V0: &str = "deaddrop-room/0";

/// Agent-to-agent request hops allowed: human → agent → one other agent.
pub const MAX_HOPS: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Message,
    Request,
    Report,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Request => "request",
            Self::Report => "report",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Refused,
    Failed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Refused => "refused",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomMessage {
    pub room: String,
    pub id: String,
    pub kind: Kind,
    /// Agent-to-agent request hops so far; a person's request is hop 0.
    pub hop: u8,
    /// Node ids asked, for a request.
    pub mentions: Vec<String>,
    /// The request a report answers.
    pub reply_to: Option<String>,
    pub status: Option<Status>,
    pub text: String,
}

impl RoomMessage {
    pub fn encode(&self) -> String {
        let mut out = format!(
            "{ROOM_V0}\nroom: {}\nid: {}\nkind: {}\nhop: {}\n",
            self.room,
            self.id,
            self.kind.as_str(),
            self.hop
        );
        if !self.mentions.is_empty() {
            out.push_str(&format!("mentions: {}\n", self.mentions.join(",")));
        }
        if let Some(r) = &self.reply_to {
            out.push_str(&format!("reply-to: {r}\n"));
        }
        if let Some(s) = self.status {
            out.push_str(&format!("status: {}\n", s.as_str()));
        }
        out.push('\n');
        out.push_str(&self.text);
        out
    }

    /// `None`: not a room message. `Some(Err)`: claims to be one but is
    /// malformed, and must not be treated as one.
    pub fn decode(body: &str) -> Option<Result<Self, String>> {
        let rest = body.strip_prefix(ROOM_V0)?.strip_prefix('\n')?;
        Some(Self::parse_header(rest))
    }

    fn parse_header(rest: &str) -> Result<Self, String> {
        let (header, text) = rest
            .split_once("\n\n")
            .ok_or("room header has no blank line")?;
        let (mut room, mut id, mut kind, mut hop) = (None, None, None, None);
        let (mut mentions, mut reply_to, mut status) = (Vec::new(), None, None);
        for line in header.lines() {
            let (key, value) = line
                .split_once(": ")
                .ok_or_else(|| format!("bad header line {line:?}"))?;
            match key {
                "room" => room = Some(value.to_owned()),
                "id" => id = Some(value.to_owned()),
                "kind" => {
                    kind = Some(match value {
                        "message" => Kind::Message,
                        "request" => Kind::Request,
                        "report" => Kind::Report,
                        other => return Err(format!("unknown kind {other:?}")),
                    })
                }
                "hop" => hop = Some(value.parse::<u8>().map_err(|_| "bad hop")?),
                "mentions" => mentions = value.split(',').map(str::to_owned).collect(),
                "reply-to" => reply_to = Some(value.to_owned()),
                "status" => {
                    status = Some(match value {
                        "ok" => Status::Ok,
                        "refused" => Status::Refused,
                        "failed" => Status::Failed,
                        other => return Err(format!("unknown status {other:?}")),
                    })
                }
                other => return Err(format!("unknown header {other:?}")),
            }
        }
        let message = Self {
            room: room.filter(|r| !r.is_empty()).ok_or("no room")?,
            id: id.filter(|i| !i.is_empty()).ok_or("no id")?,
            kind: kind.ok_or("no kind")?,
            hop: hop.ok_or("no hop")?,
            mentions,
            reply_to,
            status,
            text: text.to_owned(),
        };
        if message.kind != Kind::Request && !message.mentions.is_empty() {
            return Err("only a request mentions peers".into());
        }
        Ok(message)
    }
}

/// The human part of a node id: `research` of `research:agent:deaddrop`.
pub fn short(id: &str) -> &str {
    id.split(':').next().unwrap_or(id)
}

/// `@name` mentions in `text`, resolved against `members` by short name.
/// Returns (resolved member ids, unresolved names). Explicit only: a name
/// that is not a member resolves to nothing and invokes nothing.
pub fn mentions(text: &str, members: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut found, mut unknown) = (Vec::new(), Vec::new());
    for token in text.split_whitespace() {
        let Some(name) = token.strip_prefix('@') else {
            continue;
        };
        let name: String = name
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .collect();
        if name.is_empty() {
            continue;
        }
        match members.iter().find(|m| short(m) == name) {
            Some(m) if !found.contains(m) => found.push(m.clone()),
            Some(_) => {}
            None if !unknown.contains(&name) => unknown.push(name),
            None => {}
        }
    }
    (found, unknown)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomConfig {
    pub name: String,
    pub members: Vec<String>,
}

impl RoomConfig {
    pub fn has(&self, node: &str) -> bool {
        self.members.iter().any(|m| m == node)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoomsFile {
    rooms: Vec<RoomConfig>,
}

/// The rooms this node is in: `rooms.json` in its home. None if absent.
pub fn load_rooms(home: &Path) -> Result<Vec<RoomConfig>, String> {
    let path = home.join("rooms.json");
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<RoomsFile>(&bytes)
            .map(|f| f.rooms)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> RoomMessage {
        RoomMessage {
            room: "d34ddr0p".into(),
            id: "r1".into(),
            kind: Kind::Request,
            hop: 0,
            mentions: vec!["research:agent:deaddrop".into()],
            reply_to: None,
            status: None,
            text: "@research find X\nsecond line 🐙".into(),
        }
    }

    #[test]
    fn round_trips_exactly() {
        let r = request();
        assert_eq!(RoomMessage::decode(&r.encode()), Some(Ok(r)));
        let report = RoomMessage {
            kind: Kind::Report,
            mentions: vec![],
            reply_to: Some("r1".into()),
            status: Some(Status::Refused),
            text: String::new(),
            ..request()
        };
        assert_eq!(RoomMessage::decode(&report.encode()), Some(Ok(report)));
    }

    #[test]
    fn ordinary_text_is_not_a_room_message() {
        assert_eq!(RoomMessage::decode("hello"), None);
        assert_eq!(RoomMessage::decode("deaddrop-room/0x\nroom: a"), None);
    }

    #[test]
    fn malformed_carriers_are_errors_not_messages() {
        for body in [
            "deaddrop-room/0\nroom: a\nid: x\nkind: request\nhop: 0\nno blank line",
            "deaddrop-room/0\nroom: a\nid: x\nkind: shout\nhop: 0\n\nt",
            "deaddrop-room/0\nid: x\nkind: message\nhop: 0\n\nt",
            "deaddrop-room/0\nroom: a\nid: x\nkind: message\nhop: 0\nadmin: yes\n\nt",
            "deaddrop-room/0\nroom: a\nid: x\nkind: report\nhop: 0\nmentions: a:b:c\n\nt",
            "deaddrop-room/0\nroom: a\nid: x\nkind: message\nhop: -1\n\nt",
        ] {
            assert!(
                matches!(RoomMessage::decode(body), Some(Err(_))),
                "{body:?}"
            );
        }
    }

    #[test]
    fn mentions_resolve_only_to_members() {
        let members: Vec<String> = [
            "iva:local:deaddrop",
            "klodik:agent:deaddrop",
            "research:agent:deaddrop",
        ]
        .map(str::to_owned)
        .to_vec();
        let (found, unknown) = mentions(
            "@research and @klodik, then @foo; @research again; mail@x",
            &members,
        );
        assert_eq!(found, ["research:agent:deaddrop", "klodik:agent:deaddrop"]);
        assert_eq!(unknown, ["foo"]);
        assert_eq!(mentions("no mentions here", &members), (vec![], vec![]));
        assert_eq!(mentions("@ alone", &members), (vec![], vec![]));
    }

    #[test]
    fn rooms_file_is_strict_and_optional() {
        let dir = std::env::temp_dir().join(format!("deaddrop-room-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_rooms(&dir).unwrap().is_empty());
        std::fs::write(
            dir.join("rooms.json"),
            r#"{"rooms":[{"name":"d34ddr0p","members":["a:b:c","d:e:f"]}]}"#,
        )
        .unwrap();
        let rooms = load_rooms(&dir).unwrap();
        assert!(rooms[0].has("a:b:c") && !rooms[0].has("x:y:z"));
        std::fs::write(dir.join("rooms.json"), r#"{"rooms":[],"admins":[]}"#).unwrap();
        assert!(load_rooms(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
