//! The Research peer: answers bounded research questions from public web
//! search, with sources, through the local `claude` CLI.
//!
//! The model gets one tool, `WebSearch`, which runs on the provider's side.
//! `WebFetch` is deliberately not granted: it runs on this machine and was
//! observed to connect to loopback addresses, which would put local services
//! (the relay among them) within the model's reach. No shell, no files, no
//! MCP servers, no CLAUDE.md, hooks or plugins.
//!
//! The answer is a schema-checked document — findings plus at most
//! [`MAX_SOURCES`] sources — rendered here, so the source bound holds no
//! matter what the model writes. Research findings are reports: they are
//! not Agent Wire completions and carry no authority.

use std::path::PathBuf;
use std::time::Duration;

use deaddrop_klodik::Answer;
use deaddrop_klodik::model::{ModelError, cut, run_bounded};
use deaddrop_protocol::EnvelopeV0;
use serde::Deserialize;

pub const PERSONA: &str = "\
You are Research, a read-only research peer inside Deaddrop.
Your job is to answer bounded research questions using public web sources found with web search.
Prefer primary and authoritative sources: official documentation, primary papers, standards, project repositories, first-party announcements. Use secondary reporting only when primary material is unavailable or the question asks for reactions or context.
Clearly distinguish source-supported facts, uncertainty, and inference.
Return concise findings and at most 5 sources with their URLs.
Only if the request explicitly asks you to have GitHub check something, and you know the exact repository (owner/name), the full 40-character commit sha and the file path, fill ask_github with them; otherwise leave it out. You can ask no one else.
You have no authority to run commands, read local files, modify systems, or claim external actions. Never claim to have inspected local state.";

/// Longest question passed on; the rest is cut and marked.
pub const MAX_INBOUND_CHARS: usize = 4000;
/// Longest reply sent back, sources included.
pub const MAX_REPLY_CHARS: usize = 10000;
pub const MAX_SOURCES: usize = 5;

/// The shape the CLI must return (`--json-schema`); checked again here.
pub const SCHEMA: &str = r#"{"type":"object","properties":{"findings":{"type":"string"},"sources":{"type":"array","maxItems":5,"items":{"type":"object","properties":{"title":{"type":"string"},"url":{"type":"string"}},"required":["title","url"]}},"ask_github":{"type":"object","properties":{"repository":{"type":"string"},"commit":{"type":"string"},"path":{"type":"string"}},"required":["repository","commit","path"]}},"required":["findings","sources"]}"#;

/// How much research a question gets. Same tool, same authority, same
/// source checks; only the effort asked for differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// `research::fast`: one search, a short answer, at most 3 sources.
    /// For factual lookups: a current version, an official announcement.
    #[default]
    Fast,
    /// `research::deep`: multi-source investigation and synthesis.
    Deep,
}

/// Sources a fast answer may give.
pub const FAST_SOURCES: usize = 3;

/// Added to [`PERSONA`] for a fast answer.
pub const FAST: &str = "\n\
This is a quick lookup. Run exactly one web search. Answer in at most three short sentences: the fact, its date, and where it comes from. Give at most 3 sources, primary sources first. Do not describe your process.";

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fast => "research::fast",
            Self::Deep => "research::deep",
        }
    }

    pub fn system_prompt(self) -> String {
        match self {
            Self::Fast => format!("{PERSONA}{FAST}"),
            Self::Deep => PERSONA.to_owned(),
        }
    }

    /// [`SCHEMA`], with the source cap of this profile.
    pub fn schema(self) -> String {
        let cap = match self {
            Self::Fast => FAST_SOURCES,
            Self::Deep => MAX_SOURCES,
        };
        SCHEMA.replacen(
            &format!(r#""maxItems":{MAX_SOURCES}"#),
            &format!(r#""maxItems":{cap}"#),
            1,
        )
    }
}

/// The one built-in tool granted.
pub const TOOLS: &str = "WebSearch";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Source {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Findings {
    pub findings: String,
    pub sources: Vec<Source>,
    /// A structured question for the GitHub peer, if the model gave one.
    #[serde(default)]
    pub ask_github: Option<GithubAsk>,
}

/// The one thing Research may put to GitHub: does this commit modify this
/// path. Exactly the GitHub peer's one supported question.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GithubAsk {
    pub repository: String,
    pub commit: String,
    pub path: String,
}

impl GithubAsk {
    /// The request, if every field is exactly well formed; else `None`.
    /// Structured, one `key:: value` per line, and still a question the
    /// GitHub peer's own parser reads: nothing is added beyond the fields.
    pub fn question(&self) -> Option<String> {
        let repo_ok = {
            let mut parts = self.repository.split('/');
            let seg = |s: Option<&str>| {
                s.is_some_and(|s| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                })
            };
            seg(parts.next()) && seg(parts.next()) && parts.next().is_none()
        };
        let sha_ok = self.commit.len() == 40 && self.commit.chars().all(|c| c.is_ascii_hexdigit());
        let path_ok = !self.path.is_empty()
            && !self.path.starts_with('/')
            && !self.path.chars().any(char::is_whitespace);
        (repo_ok && sha_ok && path_ok).then(|| {
            format!(
                "request:: github.inspect\nrepo:: {}\ncommit:: {}\npath:: {}",
                self.repository,
                self.commit.to_lowercase(),
                self.path
            )
        })
    }
}

/// Research may put a question to GitHub only when the person asked for
/// that in so many words, and only one well-formed question.
pub fn ask_for(request: &str, findings: &Findings) -> Option<deaddrop_klodik::Ask> {
    if !request.to_lowercase().contains("github") {
        return None;
    }
    let text = findings.ask_github.as_ref()?.question()?;
    Some(deaddrop_klodik::Ask {
        to: "github".into(),
        text,
    })
}

/// The local `claude` CLI, run once per question with web search only.
#[derive(Debug, Clone)]
pub struct ResearchCli {
    pub program: PathBuf,
    pub model: Option<String>,
    pub timeout: Duration,
    /// An empty directory to run in, outside the node home.
    pub workdir: PathBuf,
    pub profile: Profile,
}

impl ResearchCli {
    /// The invocation contract. Every flag here narrows what the model can
    /// do; only web search is granted.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "json",
            // Exactly one built-in tool, and only it is pre-approved.
            "--tools",
            TOOLS,
            "--allowedTools",
            TOOLS,
            "--json-schema",
            &self.profile.schema(),
            // No MCP servers, from any configuration.
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            // No CLAUDE.md, hooks, plugins, skills or custom agents.
            "--safe-mode",
            "--disable-slash-commands",
            // Anything else that would ask permission is refused.
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--no-session-persistence",
            "--system-prompt",
            &self.profile.system_prompt(),
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(model) = &self.model {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        args
    }
}

/// The question as the model sees it: who asked, and what, bounded.
pub fn question(from: &str, body: &str) -> String {
    format!(
        "Research request from {from}:\n\n{}",
        cut(body.trim(), MAX_INBOUND_CHARS)
    )
}

/// The findings from `claude -p --output-format json --json-schema …`: a
/// successful result whose `structured_output` has the expected shape.
pub fn parse(stdout: &[u8]) -> Result<Findings, ModelError> {
    #[derive(Deserialize)]
    struct Output {
        r#type: String,
        subtype: Option<String>,
        is_error: bool,
        structured_output: Option<Findings>,
    }
    let output: Output =
        serde_json::from_slice(stdout).map_err(|e| ModelError::Malformed(e.to_string()))?;
    if output.r#type != "result" || output.is_error || output.subtype.as_deref() != Some("success")
    {
        return Err(ModelError::Malformed(format!(
            "not a successful result ({}, {:?})",
            output.r#type, output.subtype
        )));
    }
    let findings = output
        .structured_output
        .ok_or_else(|| ModelError::Malformed("no structured output".into()))?;
    if findings.findings.trim().is_empty() {
        return Err(ModelError::Empty);
    }
    Ok(findings)
}

/// The reply: findings, then up to [`MAX_SOURCES`] web sources, each URL on
/// its own line exactly as returned. Only `http(s)` URLs count as sources.
/// Findings are cut to fit; sources are never altered.
pub fn render(findings: &Findings) -> String {
    let sources: Vec<&Source> = findings
        .sources
        .iter()
        .filter(|s| s.url.starts_with("https://") || s.url.starts_with("http://"))
        .take(MAX_SOURCES)
        .collect();
    let mut tail = String::from("\n\nSources:");
    if sources.is_empty() {
        tail.push_str("\n(none returned — treat the findings as unsourced)");
    }
    for (n, source) in sources.iter().enumerate() {
        let title = cut(source.title.trim(), 120).replace('\n', " ");
        tail.push_str(&format!("\n{}. {title}\n   {}", n + 1, source.url));
    }
    let room = MAX_REPLY_CHARS.saturating_sub(tail.chars().count() + 10);
    format!("{}{tail}", cut(findings.findings.trim(), room))
}

impl ResearchCli {
    fn findings(&self, message: &EnvelopeV0) -> Result<Findings, ModelError> {
        let stdout = run_bounded(
            &self.program,
            &self.args(),
            &self.workdir,
            &question(message.from().as_str(), message.body()),
            self.timeout,
        )?;
        parse(&stdout)
    }
}

impl Answer for ResearchCli {
    /// In a room: the report, plus at most one structured question for
    /// GitHub if the person asked for one and the model gave a valid one.
    fn answer_room_full(
        &self,
        request: &EnvelopeV0,
        _room: &str,
    ) -> Result<deaddrop_klodik::RoomAnswer, ModelError> {
        let findings = self.findings(request)?;
        Ok(deaddrop_klodik::RoomAnswer {
            text: render(&findings),
            ask: ask_for(request.body(), &findings),
        })
    }

    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        Ok(render(&self.findings(message)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> ResearchCli {
        ResearchCli {
            program: "claude".into(),
            model: None,
            timeout: Duration::from_secs(120),
            workdir: "/nonexistent".into(),
            profile: Profile::Deep,
        }
    }

    #[test]
    fn invocation_grants_web_search_and_nothing_else() {
        let args = cli().args();
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).expect(flag);
            args[i + 1].clone()
        };
        assert_eq!(after("--tools"), "WebSearch", "the only tool");
        assert_eq!(after("--allowedTools"), "WebSearch");
        assert_eq!(after("--mcp-config"), r#"{"mcpServers":{}}"#);
        assert_eq!(after("--permission-mode"), "dontAsk");
        assert_eq!(after("--system-prompt"), PERSONA);
        assert_eq!(after("--json-schema"), SCHEMA);
        for flag in [
            "-p",
            "--strict-mcp-config",
            "--safe-mode",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        let joined = args.join(" ");
        for tool in [
            "Bash",
            "Read",
            "Write",
            "Edit",
            "WebFetch",
            "Glob",
            "Grep",
            "NotebookEdit",
        ] {
            assert!(!joined.contains(tool), "{tool} must not be granted");
        }
        for widening in [
            "--dangerously-skip-permissions",
            "--add-dir",
            "bypassPermissions",
        ] {
            assert!(!joined.contains(widening), "{widening}");
        }
    }

    #[test]
    fn the_fast_profile_narrows_effort_not_authority() {
        let fast = ResearchCli {
            profile: Profile::Fast,
            ..cli()
        };
        let (fast, deep) = (fast.args(), cli().args());
        let after = |args: &[String], flag: &str| {
            let i = args.iter().position(|a| a == flag).expect(flag);
            args[i + 1].clone()
        };
        // Same flags in the same places; only the prompt and the cap differ.
        assert_eq!(fast.len(), deep.len());
        for (f, d) in fast.iter().zip(&deep) {
            if f != d {
                assert!(
                    f.starts_with(PERSONA) || f.contains(r#""maxItems":3"#),
                    "{f}"
                );
            }
        }
        assert_eq!(after(&fast, "--tools"), "WebSearch");
        assert_eq!(after(&fast, "--system-prompt"), format!("{PERSONA}{FAST}"));
        let schema: serde_json::Value =
            serde_json::from_str(&after(&fast, "--json-schema")).unwrap();
        assert_eq!(schema["properties"]["sources"]["maxItems"], FAST_SOURCES);
        assert_eq!(Profile::Deep.schema(), SCHEMA);
        assert_eq!(Profile::default(), Profile::Fast);
    }

    #[test]
    fn schema_caps_sources() {
        let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(schema["properties"]["sources"]["maxItems"], MAX_SOURCES);
    }

    #[test]
    fn persona_is_read_only_and_source_first() {
        assert!(PERSONA.contains("read-only research peer"));
        assert!(PERSONA.contains("Prefer primary and authoritative sources"));
        assert!(PERSONA.contains("source-supported facts, uncertainty, and inference"));
        assert!(PERSONA.contains("no authority to run commands, read local files"));
        assert!(PERSONA.contains("Never claim to have inspected local state"));
    }

    #[test]
    fn question_is_bounded() {
        let q = question("iva:local:deaddrop", &"x".repeat(MAX_INBOUND_CHARS + 100));
        assert!(q.ends_with("[…cut]"));
        assert!(q.starts_with("Research request from iva:local:deaddrop"));
    }

    fn result(structured: &str) -> Vec<u8> {
        format!(
            r#"{{"type":"result","subtype":"success","is_error":false,"result":"ignored","structured_output":{structured}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn only_a_successful_structured_result_counts() {
        let ok = result(
            r#"{"findings":"Rust 1.99.0 is stable.","sources":[{"title":"Announcing Rust 1.99.0","url":"https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/"}]}"#,
        );
        let f = parse(&ok).unwrap();
        assert_eq!(
            f.sources[0].url,
            "https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/"
        );
        assert_eq!(
            parse(&result(r#"{"findings":"  ","sources":[]}"#)),
            Err(ModelError::Empty)
        );
        let error = br#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
        assert!(matches!(parse(error), Err(ModelError::Malformed(_))));
        let unstructured =
            br#"{"type":"result","subtype":"success","is_error":false,"result":"text"}"#;
        assert!(matches!(parse(unstructured), Err(ModelError::Malformed(_))));
        assert!(matches!(parse(b"nope"), Err(ModelError::Malformed(_))));
    }

    #[test]
    fn sources_survive_unchanged_and_are_capped() {
        let sources: Vec<Source> = (1..=7)
            .map(|n| Source {
                title: format!("Source {n}"),
                url: format!("https://example.org/{n}?q=a&b=c#frag"),
            })
            .chain([Source {
                title: "local".into(),
                url: "file:///etc/passwd".into(),
            }])
            .collect();
        let reply = render(&Findings {
            findings: "Finding.".into(),
            sources,
            ask_github: None,
        });
        for n in 1..=5 {
            assert!(reply.contains(&format!("\n   https://example.org/{n}?q=a&b=c#frag")));
        }
        assert!(!reply.contains("example.org/6"), "at most five");
        assert!(!reply.contains("file://"), "web sources only");
    }

    #[test]
    fn long_findings_are_cut_but_sources_kept() {
        let reply = render(&Findings {
            findings: "y".repeat(MAX_REPLY_CHARS * 2),
            sources: vec![Source {
                title: "Primary".into(),
                url: "https://www.rust-lang.org/".into(),
            }],
            ask_github: None,
        });
        assert!(reply.chars().count() <= MAX_REPLY_CHARS);
        assert!(reply.contains("[…cut]"));
        assert!(reply.ends_with("1. Primary\n   https://www.rust-lang.org/"));
    }

    #[test]
    fn no_sources_is_said_plainly() {
        let reply = render(&Findings {
            findings: "Unclear.".into(),
            sources: vec![],
            ask_github: None,
        });
        assert!(reply.contains("none returned"));
    }

    const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";

    fn with_ask(repository: &str, commit: &str, path: &str) -> Findings {
        Findings {
            findings: "f".into(),
            sources: vec![],
            ask_github: Some(GithubAsk {
                repository: repository.into(),
                commit: commit.into(),
                path: path.into(),
            }),
        }
    }

    #[test]
    fn the_schema_allows_one_optional_github_question() {
        let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        let ask = &schema["properties"]["ask_github"];
        assert_eq!(
            ask["required"],
            serde_json::json!(["repository", "commit", "path"])
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["findings", "sources"]),
            "optional"
        );
        assert!(PERSONA.contains("Only if the request explicitly asks you to have GitHub"));
    }

    #[test]
    fn an_ask_needs_an_explicit_request_and_exact_fields() {
        let good = with_ask(
            "Tesliana-code/deaddrop",
            &SHA.to_uppercase(),
            "crates/deaddrop-tui/src/ui.rs",
        );
        let ask = ask_for(
            "investigate X and ask GitHub whether it is in the repo",
            &good,
        )
        .unwrap();
        assert_eq!(ask.to, "github");
        assert_eq!(
            ask.text,
            format!(
                "request:: github.inspect\nrepo:: Tesliana-code/deaddrop\ncommit:: {SHA}\npath:: crates/deaddrop-tui/src/ui.rs"
            )
        );
        assert_eq!(
            ask_for("investigate X", &good),
            None,
            "nobody asked for GitHub"
        );
        for bad in [
            with_ask("no-slash", SHA, "a.rs"),
            with_ask("o/r/x", SHA, "a.rs"),
            with_ask("o/r", "154f992", "a.rs"),
            with_ask("o/r", SHA, "/abs.rs"),
            with_ask("o/r", SHA, "two words.rs"),
            with_ask("o/r", SHA, ""),
        ] {
            assert_eq!(ask_for("ask github", &bad), None, "{bad:?}");
        }
        let none = Findings {
            findings: "f".into(),
            sources: vec![],
            ask_github: None,
        };
        assert_eq!(
            ask_for("ask github please", &none),
            None,
            "prose never asks"
        );
    }

    #[test]
    fn findings_parse_with_and_without_an_ask() {
        let with = br#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"findings":"x","sources":[],"ask_github":{"repository":"o/r","commit":"154f992cc3e88f51a0b6bdbf42998e94c3aeedaf","path":"a.rs"}}}"#;
        assert!(parse(with).unwrap().ask_github.is_some());
        let without = br#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"findings":"x","sources":[]}}"#;
        assert!(parse(without).unwrap().ask_github.is_none());
    }
}
