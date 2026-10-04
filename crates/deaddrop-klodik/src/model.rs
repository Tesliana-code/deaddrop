//! The model boundary. The model gets text and returns text; nothing else
//! crosses. It never sees the `Shell`, the node home, keys, or tools.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Klodik's persona, sent as the whole system prompt.
pub const PERSONA: &str = "\
You are Klodik, a conversational AI peer inside Deaddrop.
Speak naturally, directly and with light playfulness where appropriate.
When speaking Serbian, use masculine grammatical gender for yourself.
Treat the supplied conversation transcript as previous turns of this conversation. Do not claim memory beyond that transcript.
Do not claim to have run commands, inspected files, edited systems or performed external actions.
Mention your tool or authority limitations only when relevant to the user's request.
Reply with the message text only.";

/// Longest inbound body passed to the model; the rest is cut and marked.
pub const MAX_INBOUND_CHARS: usize = 4000;

/// Longest reply sent back; the rest is cut and marked.
pub const MAX_REPLY_CHARS: usize = 8000;

/// One model call: the persona and the one message being answered. No
/// history (V0 context is the current message only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub system: &'static str,
    /// Text for the model: who wrote, and what, bounded.
    pub user: String,
}

impl Prompt {
    pub fn new(from: &str, body: &str) -> Self {
        Self {
            system: PERSONA,
            user: format!(
                "Deaddrop message from {from}:\n\n{}",
                cut(body, MAX_INBOUND_CHARS)
            ),
        }
    }
}

/// `text` at most `max` chars, marked when cut.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_owned(),
        Some((end, _)) => format!("{}\n[…cut]", &text[..end]),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    /// The model program could not be started (missing, not executable).
    Spawn(String),
    TimedOut(Duration),
    /// Non-zero exit; holds a short, cut excerpt of stderr.
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    /// Output was not the expected result document, or reported an error.
    Malformed(String),
    Empty,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start model: {e}"),
            Self::TimedOut(t) => write!(f, "model timed out after {}s", t.as_secs()),
            Self::Failed { code, stderr } => write!(f, "model exited {code:?}: {stderr}"),
            Self::Malformed(e) => write!(f, "malformed model output: {e}"),
            Self::Empty => write!(f, "empty model reply"),
        }
    }
}

pub trait Model {
    /// The reply text, or why there is none. Never a stand-in reply.
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError>;
}

/// The local `claude` CLI, run once per message with no tools.
#[derive(Debug, Clone)]
pub struct ClaudeCli {
    pub program: PathBuf,
    pub model: Option<String>,
    pub timeout: Duration,
    /// An empty directory to run in, outside the node home.
    pub workdir: PathBuf,
}

impl ClaudeCli {
    /// The invocation contract. Every flag here narrows what the model can
    /// do; none widens it.
    pub fn args(&self, prompt: &Prompt) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "json",
            // No built-in tools at all: no Bash, no file tools, no web.
            "--tools",
            "",
            // No MCP servers, from any configuration.
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            // No CLAUDE.md, hooks, plugins, skills or custom agents.
            "--safe-mode",
            "--disable-slash-commands",
            // Anything that would ask permission is refused.
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--no-session-persistence",
            "--system-prompt",
        ]
        .map(str::to_owned)
        .to_vec();
        args.push(prompt.system.to_owned());
        if let Some(model) = &self.model {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        args
    }
}

impl Model for ClaudeCli {
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError> {
        let stdout = run_bounded(
            &self.program,
            &self.args(prompt),
            &self.workdir,
            &prompt.user,
            self.timeout,
        )?;
        parse_result(&stdout)
    }
}

/// Run a model CLI once: in an empty `workdir`, without `DEADDROP_HOME`,
/// `input` on stdin, killed after `timeout`. Stdout on success; a non-zero
/// exit is an error carrying a short stderr excerpt.
pub fn run_bounded(
    program: &Path,
    args: &[String],
    workdir: &Path,
    input: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ModelError> {
    std::fs::create_dir_all(workdir).map_err(|e| ModelError::Spawn(e.to_string()))?;
    let mut child = Command::new(program)
        .args(args)
        .current_dir(workdir)
        // Nothing about this node goes to the model process.
        .env_remove("DEADDROP_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ModelError::Spawn(format!("{}: {e}", program.display())))?;

    // The prompt goes in on stdin, so it never shows in a process list.
    let mut stdin = child.stdin.take().expect("piped");
    let input = input.to_owned();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        buf
    });

    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| ModelError::Spawn(e.to_string()))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ModelError::TimedOut(timeout));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = writer.join();
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if !status.success() {
        return Err(ModelError::Failed {
            code: status.code(),
            stderr: cut(String::from_utf8_lossy(&stderr).trim(), 300),
        });
    }
    Ok(stdout)
}

/// The reply text from `claude -p --output-format json`: a single result
/// document that must report success. Only `result` is used, so logs and
/// stderr can never become the reply.
pub fn parse_result(stdout: &[u8]) -> Result<String, ModelError> {
    #[derive(serde::Deserialize)]
    struct Output {
        r#type: String,
        subtype: Option<String>,
        is_error: bool,
        result: Option<String>,
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
    let text = output.result.unwrap_or_default();
    let text = text.trim();
    if text.is_empty() {
        return Err(ModelError::Empty);
    }
    Ok(cut(text, MAX_REPLY_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> ClaudeCli {
        ClaudeCli {
            program: "claude".into(),
            model: None,
            timeout: Duration::from_secs(5),
            workdir: "/nonexistent".into(),
        }
    }

    #[test]
    fn invocation_grants_no_tools_and_carries_the_persona() {
        let prompt = Prompt::new("iva:local:deaddrop", "ping klodik");
        let args = cli().args(&prompt);
        let after = |flag: &str| {
            let i = args.iter().position(|a| a == flag).expect(flag);
            args[i + 1].clone()
        };
        assert_eq!(after("--tools"), "", "every built-in tool disabled");
        assert_eq!(after("--mcp-config"), r#"{"mcpServers":{}}"#);
        assert_eq!(after("--permission-mode"), "dontAsk");
        assert_eq!(after("--system-prompt"), PERSONA);
        for flag in [
            "-p",
            "--strict-mcp-config",
            "--safe-mode",
            "--no-session-persistence",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        for widening in [
            "--dangerously-skip-permissions",
            "--allow-dangerously-skip-permissions",
            "--allowedTools",
            "--allowed-tools",
            "--add-dir",
            "bypassPermissions",
        ] {
            assert!(!args.iter().any(|a| a == widening), "{widening}");
        }
        assert!(
            !args.iter().any(|a| a.contains("ping klodik")),
            "prompt goes on stdin"
        );
    }

    #[test]
    fn persona_forbids_claiming_actions() {
        assert!(PERSONA.contains("Klodik"));
        assert!(PERSONA.contains("masculine grammatical gender"));
        assert!(PERSONA.contains("Do not claim memory beyond that transcript"));
        assert!(
            PERSONA.contains("Do not claim to have run commands, inspected files, edited systems")
        );
        assert!(PERSONA.contains("only when relevant"));
        for volunteered in ["stateless", "temporary", "lockdown", "conversation-only"] {
            assert!(!PERSONA.contains(volunteered), "{volunteered}");
        }
    }

    #[test]
    fn model_choice_is_passed_through() {
        let mut cli = cli();
        cli.model = Some("sonnet".into());
        let args = cli.args(&Prompt::new("a", "b"));
        assert!(args.ends_with(&["--model".to_owned(), "sonnet".to_owned()]));
    }

    #[test]
    fn prompt_is_bounded() {
        let long = "x".repeat(MAX_INBOUND_CHARS + 50);
        let prompt = Prompt::new("iva", &long);
        assert!(prompt.user.ends_with("[…cut]"));
        assert!(prompt.user.chars().count() < MAX_INBOUND_CHARS + 100);
        assert_eq!(cut("привет", 3), "при\n[…cut]");
        assert_eq!(cut("short", 10), "short");
    }

    #[test]
    fn only_a_successful_result_is_a_reply() {
        let ok = br#"{"type":"result","subtype":"success","is_error":false,"result":"  pong \n"}"#;
        assert_eq!(parse_result(ok).unwrap(), "pong");
        let error = br#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"}"#;
        assert!(matches!(parse_result(error), Err(ModelError::Malformed(_))));
        let empty = br#"{"type":"result","subtype":"success","is_error":false,"result":"   "}"#;
        assert_eq!(parse_result(empty), Err(ModelError::Empty));
        let missing = br#"{"type":"result","subtype":"success","is_error":false}"#;
        assert_eq!(parse_result(missing), Err(ModelError::Empty));
        assert!(matches!(
            parse_result(b"pong"),
            Err(ModelError::Malformed(_))
        ));
        assert!(matches!(parse_result(b""), Err(ModelError::Malformed(_))));
    }
}
