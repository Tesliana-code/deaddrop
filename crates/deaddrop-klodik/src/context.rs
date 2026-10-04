//! Klodik's bounded conversational context, owned by the adapter.
//!
//! Each completed exchange — a verified inbound message and the reply that
//! was actually sent — is kept per peer in Klodik's own home, in local
//! processing order (no timestamps exist to order by). The model sees the
//! newest completed exchanges that fit, then the current message. The model
//! CLI keeps no session of its own (`--no-session-persistence`); this file
//! is the whole memory, and it is inspectable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use deaddrop_protocol::{EnvelopeV0, MessageId};
use serde::{Deserialize, Serialize};

use crate::Answer;
use crate::model::{MAX_INBOUND_CHARS, Model, ModelError, PERSONA, Prompt, cut};

/// Completed exchanges kept per peer.
pub const MAX_EXCHANGES: usize = 8;
/// Longest transcript, in characters, put in front of the current message.
pub const MAX_TRANSCRIPT_CHARS: usize = 12000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    pub message_id: String,
    pub text: String,
}

/// A verified message and the reply that was sent to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub user: Turn,
    pub assistant: Turn,
}

/// One line said in a room, by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomLine {
    pub author: String,
    pub text: String,
}

/// Room lines kept per room.
pub const MAX_ROOM_LINES: usize = 30;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranscriptFile {
    peers: BTreeMap<String, Vec<Exchange>>,
    #[serde(default)]
    rooms: BTreeMap<String, Vec<RoomLine>>,
}

/// Completed exchanges by peer id: one small JSON file, replaced atomically.
#[derive(Debug)]
pub struct Transcripts {
    path: PathBuf,
    file: TranscriptFile,
}

impl Transcripts {
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => TranscriptFile::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(Self {
            path: path.to_owned(),
            file,
        })
    }

    /// Oldest first.
    pub fn exchanges(&self, peer: &str) -> &[Exchange] {
        self.file.peers.get(peer).map_or(&[], Vec::as_slice)
    }

    /// Oldest first.
    pub fn room(&self, room: &str) -> &[RoomLine] {
        self.file.rooms.get(room).map_or(&[], Vec::as_slice)
    }

    /// Keep a room line, dropping the oldest past [`MAX_ROOM_LINES`].
    pub fn record_room(&mut self, room: &str, line: RoomLine) -> Result<(), String> {
        let list = self.file.rooms.entry(room.to_owned()).or_default();
        list.push(line);
        let excess = list.len().saturating_sub(MAX_ROOM_LINES);
        list.drain(..excess);
        self.save()
    }

    /// Keep `exchange` with `peer`, once, dropping the oldest past
    /// [`MAX_EXCHANGES`].
    pub fn record(&mut self, peer: &str, exchange: Exchange) -> Result<(), String> {
        let list = self.file.peers.entry(peer.to_owned()).or_default();
        if list
            .iter()
            .any(|e| e.user.message_id == exchange.user.message_id)
        {
            return Ok(());
        }
        list.push(exchange);
        let excess = list.len().saturating_sub(MAX_EXCHANGES);
        list.drain(..excess);
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(&self.file).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }
}

/// How a speaker is named in the prompt: the first segment of the node id.
pub fn speaker(id: &str) -> &str {
    match id.find(':') {
        Some(i) if i > 0 => &id[..i],
        _ => id,
    }
}

fn turn(who: &str, text: &str) -> String {
    let body: Vec<String> = text.lines().map(|l| format!("  {l}")).collect();
    format!("{who}:\n{}\n", body.join("\n"))
}

/// The model's input: the newest completed exchanges that fit in
/// [`MAX_TRANSCRIPT_CHARS`], whole and attributed, then the current message.
pub fn render(own: &str, peer: &str, history: &[Exchange], current: &str) -> String {
    let (me, them) = (speaker(own), speaker(peer));
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    for exchange in history.iter().rev().take(MAX_EXCHANGES) {
        let block = format!(
            "{}\n{}",
            turn(them, &exchange.user.text),
            turn(me, &exchange.assistant.text)
        );
        let size = block.chars().count() + 1;
        if used + size > MAX_TRANSCRIPT_CHARS {
            break;
        }
        used += size;
        kept.push(block);
    }
    kept.reverse();

    let mut out = String::new();
    if !kept.is_empty() {
        out.push_str("CONVERSATION SO FAR\n\n");
        out.push_str(&kept.join("\n"));
        out.push('\n');
    }
    let current = cut(current.trim(), MAX_INBOUND_CHARS);
    let current: Vec<String> = current.lines().map(|l| format!("  {l}")).collect();
    out.push_str(&format!(
        "CURRENT MESSAGE FROM {them}\n\n{}",
        current.join("\n")
    ));
    out
}

/// A room request for the model: the newest room lines that fit in
/// [`MAX_TRANSCRIPT_CHARS`], each attributed, then the request.
pub fn render_room(room: &str, lines: &[RoomLine], from: &str, current: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    for line in lines.iter().rev() {
        let block = turn(
            speaker(&line.author),
            &cut(line.text.trim(), MAX_INBOUND_CHARS),
        );
        let size = block.chars().count() + 1;
        if used + size > MAX_TRANSCRIPT_CHARS {
            break;
        }
        used += size;
        kept.push(block);
    }
    kept.reverse();
    let mut out = String::new();
    if !kept.is_empty() {
        out.push_str(&format!("ROOM #{room} SO FAR\n\n"));
        out.push_str(&kept.join("\n"));
        out.push('\n');
    }
    let current = cut(current.trim(), MAX_INBOUND_CHARS);
    let current: Vec<String> = current.lines().map(|l| format!("  {l}")).collect();
    out.push_str(&format!(
        "REQUEST FROM {} IN #{room}\n\n{}",
        speaker(from),
        current.join("\n")
    ));
    out
}

/// A model with per-peer conversational context.
pub struct Conversational<M> {
    pub model: M,
    pub transcripts: Transcripts,
    /// This node's id, for naming its own turns.
    pub own: String,
}

impl<M: Model> Answer for Conversational<M> {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        let peer = message.from().as_str();
        let prompt = Prompt {
            system: PERSONA,
            user: render(
                &self.own,
                peer,
                self.transcripts.exchanges(peer),
                message.body(),
            ),
        };
        self.model.reply(&prompt)
    }

    fn answer_room(&self, request: &EnvelopeV0, room: &str) -> Result<String, ModelError> {
        let prompt = Prompt {
            system: PERSONA,
            user: render_room(
                room,
                self.transcripts.room(room),
                request.from().as_str(),
                request.body(),
            ),
        };
        self.model.reply(&prompt)
    }

    fn observe_room(&mut self, room: &str, author: &str, text: &str) -> Result<(), String> {
        self.transcripts.record_room(
            room,
            RoomLine {
                author: author.to_owned(),
                text: cut(text.trim(), MAX_INBOUND_CHARS),
            },
        )
    }

    fn replied(
        &mut self,
        message: &EnvelopeV0,
        reply: &str,
        reply_id: &MessageId,
    ) -> Result<(), String> {
        self.transcripts.record(
            message.from().as_str(),
            Exchange {
                user: Turn {
                    message_id: message.id().to_string(),
                    text: cut(message.body().trim(), MAX_INBOUND_CHARS),
                },
                assistant: Turn {
                    message_id: reply_id.to_string(),
                    text: reply.to_owned(),
                },
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: &str = "klodik:agent:deaddrop";
    const IVA: &str = "iva:local:deaddrop";

    fn exchange(n: usize, user: &str, reply: &str) -> Exchange {
        Exchange {
            user: Turn {
                message_id: format!("u{n}"),
                text: user.into(),
            },
            assistant: Turn {
                message_id: format!("r{n}"),
                text: reply.into(),
            },
        }
    }

    #[test]
    fn first_message_has_no_transcript() {
        let text = render(ME, IVA, &[], "zdravo");
        assert_eq!(text, "CURRENT MESSAGE FROM iva\n\n  zdravo");
        assert!(!text.contains("CONVERSATION SO FAR"));
    }

    #[test]
    fn history_comes_first_with_both_speakers_named() {
        let history = [exchange(
            1,
            "moja omiljena životinja je hobotnica",
            "Hobotnica! Moja je vidra.",
        )];
        let text = render(ME, IVA, &history, "a moja?");
        assert_eq!(
            text,
            "CONVERSATION SO FAR\n\n\
             iva:\n  moja omiljena životinja je hobotnica\n\n\
             klodik:\n  Hobotnica! Moja je vidra.\n\n\
             CURRENT MESSAGE FROM iva\n\n  a moja?"
        );
    }

    #[test]
    fn multiline_turns_stay_indented_under_their_speaker() {
        let history = [exchange(1, "one\ntwo", "three\nfour")];
        let text = render(ME, IVA, &history, "five\nsix");
        assert!(text.contains("iva:\n  one\n  two\n"));
        assert!(text.contains("klodik:\n  three\n  four\n"));
        assert!(text.ends_with("CURRENT MESSAGE FROM iva\n\n  five\n  six"));
    }

    #[test]
    fn only_the_newest_whole_exchanges_that_fit_are_kept() {
        let long = "x".repeat(5000);
        let history: Vec<Exchange> = (1..=4)
            .map(|n| exchange(n, &format!("q{n} {long}"), &format!("a{n}")))
            .collect();
        let text = render(ME, IVA, &history, "now");
        assert!(text.contains("q4 ") && text.contains("q3 "));
        assert!(
            !text.contains("q2 ") && !text.contains("q1 "),
            "older dropped whole"
        );
        let transcript = &text[..text.find("CURRENT MESSAGE").unwrap()];
        assert!(transcript.chars().count() <= MAX_TRANSCRIPT_CHARS + 40);
        // Every kept turn still names its speaker.
        let headers = |who: &str| text.lines().filter(|l| *l == who).count();
        assert_eq!((headers("iva:"), headers("klodik:")), (2, 2));
    }

    #[test]
    fn an_exchange_too_big_to_fit_is_left_out_not_cut() {
        let history = [exchange(1, &"y".repeat(MAX_TRANSCRIPT_CHARS), "ok")];
        let text = render(ME, IVA, &history, "hi");
        assert_eq!(text, "CURRENT MESSAGE FROM iva\n\n  hi");
    }

    #[test]
    fn at_most_eight_exchanges_render() {
        let history: Vec<Exchange> = (1..=12)
            .map(|n| exchange(n, &format!("q{n}."), "ok"))
            .collect();
        let text = render(ME, IVA, &history, "now");
        assert!(!text.contains("q4.") && text.contains("q5.") && text.contains("q12."));
    }

    #[test]
    fn records_are_per_peer_bounded_and_kept_once() {
        let dir = std::env::temp_dir().join(format!("klodik-ctx-{}", std::process::id()));
        let path = dir.join("transcripts.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut t = Transcripts::load(&path).unwrap();
        for n in 1..=10 {
            t.record(IVA, exchange(n, &format!("q{n}"), "a")).unwrap();
        }
        t.record(IVA, exchange(10, "q10", "a")).unwrap();
        t.record("danil:local:deaddrop", exchange(99, "hej", "hej"))
            .unwrap();
        let ids: Vec<&str> = t
            .exchanges(IVA)
            .iter()
            .map(|e| e.user.message_id.as_str())
            .collect();
        assert_eq!(ids, ["u3", "u4", "u5", "u6", "u7", "u8", "u9", "u10"]);
        assert_eq!(t.exchanges("danil:local:deaddrop").len(), 1);
        assert!(t.exchanges("nobody").is_empty());
        let again = Transcripts::load(&path).unwrap();
        assert_eq!(again.exchanges(IVA), t.exchanges(IVA), "survives a reload");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn room_requests_see_who_said_what() {
        let lines = [
            RoomLine {
                author: "iva:local:deaddrop".into(),
                text: "@research find the latest Rust release".into(),
            },
            RoomLine {
                author: "research:agent:deaddrop".into(),
                text: "Rust 1.99.0, 2026-10-01".into(),
            },
        ];
        let text = render_room(
            "d34ddr0p",
            &lines,
            "iva:local:deaddrop",
            "@klodik summarize",
        );
        assert_eq!(
            text,
            "ROOM #d34ddr0p SO FAR\n\n\
             iva:\n  @research find the latest Rust release\n\n\
             research:\n  Rust 1.99.0, 2026-10-01\n\n\
             REQUEST FROM iva IN #d34ddr0p\n\n  @klodik summarize"
        );
        assert_eq!(
            render_room("d34ddr0p", &[], "iva:local:deaddrop", "hi"),
            "REQUEST FROM iva IN #d34ddr0p\n\n  hi"
        );
        let long: Vec<RoomLine> = (0..10)
            .map(|n| RoomLine {
                author: "x:y:z".into(),
                text: format!("line{n} {}", "w".repeat(3000)),
            })
            .collect();
        let text = render_room("r", &long, "a:b:c", "q");
        assert!(
            text.contains("line9") && !text.contains("line0"),
            "newest that fit"
        );
    }

    #[test]
    fn room_lines_are_bounded_and_survive_reload() {
        let dir = std::env::temp_dir().join(format!("klodik-room-{}", std::process::id()));
        let path = dir.join("transcripts.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut t = Transcripts::load(&path).unwrap();
        for n in 0..(MAX_ROOM_LINES + 5) {
            t.record_room(
                "d34ddr0p",
                RoomLine {
                    author: "a:b:c".into(),
                    text: format!("{n}"),
                },
            )
            .unwrap();
        }
        let again = Transcripts::load(&path).unwrap();
        assert_eq!(again.room("d34ddr0p").len(), MAX_ROOM_LINES);
        assert_eq!(again.room("d34ddr0p")[0].text, "5");
        assert!(again.room("other").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
