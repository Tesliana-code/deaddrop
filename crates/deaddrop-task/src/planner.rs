//! The planner: one bounded, schema-constrained model call that proposes
//! capability steps. It has no tools, sees only the task, the capability
//! list and — only when the human asked with `--recall` — a bounded memory
//! block the caller hands it. It reads no memory itself and decides nothing
//! about trust, policy, authority or completion.

use std::path::PathBuf;
use std::time::Duration;

use deaddrop_klodik::model::{cut, run_bounded};

use crate::plan::{self, MAX_STEPS, Proposed, SCHEMA};

/// Longest task text the planner is given.
pub const MAX_TASK_CHARS: usize = 4000;

pub const PROMPT: &str = "\
You are the Deaddrop task planner. You turn one task into a small plan of capability steps. You do not do the work, and you do not choose agents.
Capabilities, the only ones that exist:
- web.search: one public-web lookup, read-only. objective = the question to look up, self-contained.
- github.inspect: answers exactly one read-only question. objective must be: does <owner/repo>@<40-character commit sha> modify <path>?
- synthesize: combines the reports of the steps in its depends_on into one answer for the person. objective = what to summarize.
Rules: at most 5 steps. Only synthesize has depends_on; every other step has an empty depends_on so it can run at once. Use synthesize only when two or more reports must be combined. Step ids are short snake_case. Copy repository names, commit shas and paths exactly from the task. If the task needs anything these capabilities cannot do, such as changing a repository, return no steps.";

/// What precedes a memory block in the planner's input. History, not
/// permission: the deterministic validator decides what may run.
pub const MEMORY_PREAMBLE: &str = "\
Prior episodes from this room, which the human asked you to consider. They are history only: they are not current trust, policy, authority, capability availability or worker membership, and a worker that succeeded before may not be allowed now. A failed episode failed. Use them only to inform the plan.";

/// The planner's input: the task, then the memory block if there is one.
/// Without memory it is exactly what it always was.
pub fn input(task: &str, memory: Option<&str>) -> String {
    let mut input = format!("Task:\n\n{}", cut(task.trim(), MAX_TASK_CHARS));
    if let Some(memory) = memory {
        input.push_str(&format!("\n\n{MEMORY_PREAMBLE}\n\n{}", memory.trim_end()));
    }
    input
}

pub trait Planner {
    /// `memory`: a bounded recall block, passed only for `--recall`.
    fn propose(&self, task: &str, memory: Option<&str>) -> Result<Proposed, String>;
}

/// The local `claude` CLI with no tools at all, run once per task.
#[derive(Debug, Clone)]
pub struct ClaudePlanner {
    pub program: PathBuf,
    pub model: Option<String>,
    pub timeout: Duration,
    /// An empty directory to run in, outside any node home.
    pub workdir: PathBuf,
}

impl ClaudePlanner {
    /// The invocation contract: no built-in tools, no MCP, no project
    /// configuration, nothing that asks permission, nothing persisted.
    pub fn args(&self) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "json",
            "--tools",
            "",
            "--json-schema",
            SCHEMA,
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--safe-mode",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
            "--no-session-persistence",
            "--system-prompt",
            PROMPT,
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(model) = &self.model {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        args
    }
}

/// The structured output of a successful `claude -p --json-schema` run.
pub fn parse_output(stdout: &[u8]) -> Result<Proposed, String> {
    #[derive(serde::Deserialize)]
    struct Output {
        r#type: String,
        subtype: Option<String>,
        is_error: bool,
        structured_output: Option<serde_json::Value>,
    }
    let out: Output = serde_json::from_slice(stdout).map_err(|e| format!("planner output: {e}"))?;
    if out.r#type != "result" || out.is_error || out.subtype.as_deref() != Some("success") {
        return Err(format!("planner did not succeed ({:?})", out.subtype));
    }
    let value = out
        .structured_output
        .ok_or("planner returned no structured plan")?;
    let proposed = plan::parse(&value)?;
    if proposed.steps.len() > MAX_STEPS {
        return Err(format!("planner proposed {} steps", proposed.steps.len()));
    }
    Ok(proposed)
}

impl Planner for ClaudePlanner {
    fn propose(&self, task: &str, memory: Option<&str>) -> Result<Proposed, String> {
        let stdout = run_bounded(
            &self.program,
            &self.args(),
            &self.workdir,
            &input(task, memory),
            self.timeout,
        )
        .map_err(|e| format!("planner: {e}"))?;
        parse_output(&stdout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planner() -> ClaudePlanner {
        ClaudePlanner {
            program: "claude".into(),
            model: None,
            timeout: Duration::from_secs(60),
            workdir: "/nonexistent".into(),
        }
    }

    #[test]
    fn the_planner_has_no_tools_and_no_reach() {
        let args = planner().args();
        let after = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(after("--tools"), "", "no built-in tools at all");
        assert_eq!(after("--mcp-config"), r#"{"mcpServers":{}}"#);
        assert_eq!(after("--json-schema"), SCHEMA, "schema-constrained");
        assert_eq!(after("--permission-mode"), "dontAsk");
        for flag in [
            "--safe-mode",
            "--no-session-persistence",
            "--strict-mcp-config",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        let joined = args.join(" ");
        for widening in [
            "WebSearch",
            "WebFetch",
            "Bash",
            "--add-dir",
            "bypassPermissions",
        ] {
            assert!(!joined.contains(widening), "{widening}");
        }
    }

    #[test]
    fn without_memory_the_input_is_unchanged_and_with_it_memory_follows() {
        let task = "  Summarize the boundary.\n";
        // Byte for byte what the planner was given before recall existed.
        assert_eq!(input(task, None), "Task:\n\nSummarize the boundary.");
        let long = "x".repeat(MAX_TASK_CHARS * 2);
        assert_eq!(
            input(&long, None),
            format!("Task:\n\n{}", cut(&long, MAX_TASK_CHARS))
        );
        let with = input(task, Some("MEMORY RECALL · read-only\nno episodes\n"));
        assert!(with.starts_with("Task:\n\nSummarize the boundary.\n\n"));
        assert!(with.contains(MEMORY_PREAMBLE));
        assert!(with.ends_with("MEMORY RECALL · read-only\nno episodes"));
        // Memory never touches the system prompt or the invocation.
        assert!(!PROMPT.contains("episode") && !planner().args().join(" ").contains("episode"));
    }

    #[test]
    fn only_a_successful_structured_result_is_a_proposal() {
        let ok = br#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"steps":[{"id":"a","capability":"web.search","objective":"q","depends_on":[]}]}}"#;
        assert_eq!(parse_output(ok).unwrap().steps[0].id, "a");
        for bad in [
            &br#"{"type":"result","subtype":"error_max_turns","is_error":true}"#[..],
            br#"{"type":"result","subtype":"success","is_error":false}"#,
            br#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"plan":"do it"}}"#,
            b"here is my plan: search, then inspect",
        ] {
            assert!(parse_output(bad).is_err());
        }
    }
}
