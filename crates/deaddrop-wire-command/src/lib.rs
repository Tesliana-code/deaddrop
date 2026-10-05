//! Wire Command Language V0: the deterministic control syntax inside a
//! d34ddr0p message. See `docs/wire-command-language-v0.md`.
//!
//! [`parse`] reads one message and says exactly one of:
//!
//! - [`Parsed::Ordinary`]: chat text; nothing to control.
//! - [`Parsed::Command`]: a valid [`WireCommand`].
//! - [`Parsed::Invalid`]: the message ends in control syntax that is wrong.
//!   It must not be sent as chat.
//!
//! Only the final non-empty line can be a command; everything above it is
//! the payload. No model, no fuzzy matching, no aliases.
//!
//! A [`WireCommand`] is user *intent*. Parsing grants no trust, no policy
//! permission and no authority; a valid command may still be refused later.

use std::fmt;

/// One parsed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Ordinary,
    Command(WireCommand),
    Invalid(WireError),
}

/// What the user asked the orchestrator for. Data only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireCommand {
    /// A goal; the orchestrator decides structure.
    Objective {
        payload: String,
        review: bool,
        verbose: bool,
    },
    /// A concrete task to route and schedule, not reinterpret.
    Task {
        payload: String,
        dry_run: bool,
        /// The human asks the orchestrator to consult this room's recent
        /// episodes while planning. Advisory input only: it grants nothing.
        recall: bool,
    },
    /// Read-only evidence gathering with the named capabilities only.
    Inspect {
        payload: String,
        require: Vec<CapabilityId>,
    },
    Status {
        id: Option<WireExecutionId>,
    },
    Trace {
        id: Option<WireExecutionId>,
    },
    /// Stop future work. Not rollback.
    Cancel {
        id: WireExecutionId,
    },
}

/// The most a command can ever lead to. There is deliberately no variant
/// for "granted": authority comes from policy and grants, never syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideEffects {
    /// Nothing may change in the world because of this command.
    Never,
    /// Changes only as later policy and explicit authority allow.
    SubjectToAuthority,
}

impl WireCommand {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Objective { .. } => "objective",
            Self::Task { .. } => "task",
            Self::Inspect { .. } => "inspect",
            Self::Status { .. } => "status",
            Self::Trace { .. } => "trace",
            Self::Cancel { .. } => "cancel",
        }
    }

    /// The ceiling this command's own semantics put on side effects.
    pub fn side_effects(&self) -> SideEffects {
        match self {
            Self::Task { dry_run: true, .. }
            | Self::Inspect { .. }
            | Self::Status { .. }
            | Self::Trace { .. } => SideEffects::Never,
            // Cancel stops future work; it never performs or undoes any.
            Self::Cancel { .. } => SideEffects::Never,
            Self::Objective { .. } | Self::Task { .. } => SideEffects::SubjectToAuthority,
        }
    }
}

/// A capability name: dot-separated segments of `a-z`, `0-9`, `_`, `-`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(String);

impl CapabilityId {
    pub fn parse(value: &str) -> Result<Self, WireError> {
        let segment_ok = |s: &str| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        };
        if value.split('.').all(segment_ok) {
            Ok(Self(value.to_owned()))
        } else {
            Err(WireError::new(format!("malformed capability '{value}'")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An execution, task or objective id, as the orchestrator will issue them.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WireExecutionId(String);

impl WireExecutionId {
    pub const MAX_LEN: usize = 128;

    pub fn parse(value: &str) -> Result<Self, WireError> {
        let ok = !value.is_empty()
            && value.len() <= Self::MAX_LEN
            && !value.starts_with('-')
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'));
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(WireError::new(format!("malformed id '{value}'")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A CLI-style error: `wire: <what is wrong>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError(String);

impl WireError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The message without the `wire: ` prefix.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "wire: {}", self.0)
    }
}

impl std::error::Error for WireError {}

const COMMANDS: [&str; 6] = ["objective", "task", "inspect", "status", "trace", "cancel"];
const FLAGS: [&str; 5] = ["review", "verbose", "dry-run", "recall", "require"];

/// Parse one message. Total: never panics, for any input.
pub fn parse(message: &str) -> Parsed {
    let lines: Vec<&str> = message.split('\n').collect();
    let Some(last) = lines.iter().rposition(|l| !l.trim().is_empty()) else {
        return Parsed::Ordinary;
    };
    let line = lines[last].trim();
    let mut tokens = line.split_whitespace();
    let head = tokens.next().unwrap_or_default();

    // Command-shaped: a leading `/` and `::wire` (any case) in the first
    // token. Shaped but not exact is an error, never chat; anything else is
    // chat, however much it resembles a command.
    if !(head.starts_with('/') && head.to_ascii_lowercase().contains("::wire")) {
        return Parsed::Ordinary;
    }
    let result = command(head, tokens.collect(), payload(&lines[..last]));
    match result {
        Ok(command) => Parsed::Command(command),
        Err(error) => Parsed::Invalid(error),
    }
}

/// The text above the command line: leading and trailing blank lines and
/// trailing whitespace removed; everything between kept byte for byte.
fn payload(lines: &[&str]) -> String {
    let first = lines.iter().position(|l| !l.trim().is_empty());
    match first {
        None => String::new(),
        Some(first) => lines[first..].join("\n").trim_end().to_owned(),
    }
}

fn command(head: &str, args: Vec<&str>, payload: String) -> Result<WireCommand, WireError> {
    let name = head
        .strip_prefix('/')
        .and_then(|h| h.strip_suffix("::wire"))
        .filter(|n| !n.is_empty() && !n.contains(':'))
        .ok_or_else(|| WireError::new(format!("malformed command '{head}'")))?;
    if name == "help" {
        return Err(WireError::new("/help::wire is reserved for V1"));
    }
    if !COMMANDS.contains(&name) {
        return Err(WireError::new(format!("unknown command '{head}'")));
    }

    let mut flags = Flags::default();
    let mut positional: Vec<&str> = Vec::new();
    for arg in args {
        match arg.strip_prefix("--") {
            Some(flag) => flags.add(name, flag)?,
            None if arg.starts_with('-') => {
                return Err(WireError::new(format!("unknown flag '{arg}'")));
            }
            None => positional.push(arg),
        }
    }

    match name {
        "objective" | "task" | "inspect" => {
            if let Some(arg) = positional.first() {
                return Err(WireError::new(format!(
                    "{name} takes no arguments; put the text above the command ('{arg}')"
                )));
            }
            if payload.is_empty() {
                return Err(WireError::new(format!("{name} requires payload")));
            }
            Ok(match name {
                "objective" => WireCommand::Objective {
                    payload,
                    review: flags.review,
                    verbose: flags.verbose,
                },
                "task" => WireCommand::Task {
                    payload,
                    dry_run: flags.dry_run,
                    recall: flags.recall,
                },
                _ => {
                    if flags.require.is_empty() {
                        return Err(WireError::new("inspect requires at least one capability"));
                    }
                    WireCommand::Inspect {
                        payload,
                        require: flags.require,
                    }
                }
            })
        }
        _ => {
            if !payload.is_empty() {
                return Err(WireError::new(format!(
                    "{name} takes no text; send it as its own message"
                )));
            }
            let id = match positional.as_slice() {
                [] => None,
                [id] => Some(WireExecutionId::parse(id)?),
                _ => return Err(WireError::new(format!("{name} takes at most one <id>"))),
            };
            Ok(match (name, id) {
                ("status", id) => WireCommand::Status { id },
                ("trace", id) => WireCommand::Trace { id },
                (_, Some(id)) => WireCommand::Cancel { id },
                (_, None) => return Err(WireError::new("cancel requires <id>")),
            })
        }
    }
}

/// Flags seen on one command line. Each may appear once.
#[derive(Default)]
struct Flags {
    seen: Vec<String>,
    review: bool,
    verbose: bool,
    dry_run: bool,
    recall: bool,
    require: Vec<CapabilityId>,
}

impl Flags {
    fn add(&mut self, command: &str, flag: &str) -> Result<(), WireError> {
        let (key, value) = match flag.split_once('=') {
            Some((key, value)) => (key, Some(value)),
            None => (flag, None),
        };
        if !FLAGS.contains(&key) {
            return Err(WireError::new(format!("unknown flag '--{flag}'")));
        }
        let allowed = match command {
            "objective" => ["review", "verbose"].contains(&key),
            "task" => ["dry-run", "recall"].contains(&key),
            "inspect" => key == "require",
            _ => false,
        };
        if !allowed {
            return Err(WireError::new(format!(
                "{command} does not accept '--{key}'"
            )));
        }
        // Duplicates are rejected, not merged: one spelling, one meaning.
        if self.seen.iter().any(|s| s == key) {
            return Err(WireError::new(format!("duplicate flag '--{key}'")));
        }
        self.seen.push(key.to_owned());

        match (key, value) {
            ("require", None) => Err(WireError::new("--require needs a value")),
            ("require", Some(list)) => {
                if list.is_empty() {
                    return Err(WireError::new("inspect requires at least one capability"));
                }
                for item in list.split(',') {
                    if item.is_empty() {
                        return Err(WireError::new(format!(
                            "malformed capability list '{list}'"
                        )));
                    }
                    let capability = CapabilityId::parse(item)?;
                    if self.require.contains(&capability) {
                        return Err(WireError::new(format!("duplicate capability '{item}'")));
                    }
                    self.require.push(capability);
                }
                Ok(())
            }
            (_, Some(_)) => Err(WireError::new(format!("flag '--{key}' takes no value"))),
            ("review", None) => {
                self.review = true;
                Ok(())
            }
            ("verbose", None) => {
                self.verbose = true;
                Ok(())
            }
            ("recall", None) => {
                self.recall = true;
                Ok(())
            }
            (_, None) => {
                self.dry_run = true;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(message: &str) -> WireCommand {
        match parse(message) {
            Parsed::Command(command) => command,
            other => panic!("{message:?}: {other:?}"),
        }
    }

    fn error(message: &str) -> String {
        match parse(message) {
            Parsed::Invalid(error) => error.to_string(),
            other => panic!("{message:?}: {other:?}"),
        }
    }

    fn cap(value: &str) -> CapabilityId {
        CapabilityId::parse(value).unwrap()
    }

    fn id(value: &str) -> WireExecutionId {
        WireExecutionId::parse(value).unwrap()
    }

    #[test]
    fn t01_ordinary_chat_stays_ordinary() {
        for text in [
            "",
            "   \n\n",
            "zdravo klodik",
            "a/b/c path",
            "/home/superadmin is my home",
            "see https://example.org/x::wire",
            "🌼 cveće",
        ] {
            assert_eq!(parse(text), Parsed::Ordinary, "{text:?}");
        }
    }

    #[test]
    fn t02_only_the_final_non_empty_line_activates() {
        assert_eq!(
            command("check this\n/task::wire\n\n  \n"),
            WireCommand::Task {
                payload: "check this".into(),
                dry_run: false,
                recall: false
            }
        );
        assert_eq!(
            parse("/task::wire --dry-run\nand then more text"),
            Parsed::Ordinary
        );
    }

    #[test]
    fn t03_command_text_in_the_middle_is_chat() {
        let text = "you can use /task::wire like this:\n/objective::wire --review\nsee?";
        assert_eq!(parse(text), Parsed::Ordinary);
        assert_eq!(parse("you can use /task::wire"), Parsed::Ordinary);
    }

    #[test]
    fn t04_multiline_objective_payload_is_preserved() {
        let text = "Proveri da li novi IBM RAG pristup ima nešto\n  što možemo primeniti na TriageAI,\n\nuporedi sa repo obrascima.\n\n/objective::wire --review --verbose";
        let WireCommand::Objective { payload, .. } = command(text) else {
            panic!()
        };
        assert_eq!(
            payload,
            "Proveri da li novi IBM RAG pristup ima nešto\n  što možemo primeniti na TriageAI,\n\nuporedi sa repo obrascima."
        );
    }

    #[test]
    fn t05_t06_t07_objective_flags_in_any_order() {
        let parsed = |flags: &str| command(&format!("goal\n/objective::wire{flags}"));
        let objective = |review, verbose| WireCommand::Objective {
            payload: "goal".into(),
            review,
            verbose,
        };
        assert_eq!(parsed(""), objective(false, false));
        assert_eq!(parsed(" --review"), objective(true, false));
        assert_eq!(parsed(" --verbose"), objective(false, true));
        assert_eq!(parsed(" --review --verbose"), objective(true, true));
        assert_eq!(parsed(" --verbose --review"), objective(true, true));
    }

    #[test]
    fn t08_task_dry_run() {
        assert_eq!(
            command("Inspect commit X.\n/task::wire --dry-run"),
            WireCommand::Task {
                payload: "Inspect commit X.".into(),
                dry_run: true,
                recall: false
            }
        );
    }

    #[test]
    fn task_recall_is_opt_in_and_order_free() {
        let parsed = |flags: &str| command(&format!("t\n/task::wire{flags}"));
        let task = |dry_run, recall| WireCommand::Task {
            payload: "t".into(),
            dry_run,
            recall,
        };
        assert_eq!(parsed(""), task(false, false));
        assert_eq!(parsed(" --recall"), task(false, true));
        assert_eq!(parsed(" --dry-run"), task(true, false));
        assert_eq!(parsed(" --dry-run --recall"), task(true, true));
        assert_eq!(parsed(" --recall --dry-run"), task(true, true));
        // Recall names no scope: the room is the orchestrator's to decide.
        for bad in [
            ("--recall=yes", "wire: flag '--recall' takes no value"),
            ("--recall --recall", "wire: duplicate flag '--recall'"),
            ("--recall --room", "wire: unknown flag '--room'"),
            (
                "--recall --scope=peer/x",
                "wire: unknown flag '--scope=peer/x'",
            ),
            (
                "--recall --caller=operator",
                "wire: unknown flag '--caller=operator'",
            ),
            ("--recall --project=x", "wire: unknown flag '--project=x'"),
            (
                "--recall room/b4ckr00m",
                "wire: task takes no arguments; put the text above the command ('room/b4ckr00m')",
            ),
        ] {
            assert_eq!(
                error(&format!("t\n/task::wire {}", bad.0)),
                bad.1,
                "{bad:?}"
            );
        }
        assert_eq!(
            error("g\n/objective::wire --recall"),
            "wire: objective does not accept '--recall'"
        );
        // Recall does not raise the side-effect ceiling either way.
        assert_eq!(
            parsed(" --dry-run --recall").side_effects(),
            SideEffects::Never
        );
        assert_eq!(
            parsed(" --recall").side_effects(),
            SideEffects::SubjectToAuthority
        );
    }

    #[test]
    fn t09_t10_inspect_capabilities() {
        assert_eq!(
            command("Check commit 154f.\n/inspect::wire --require=github.inspect"),
            WireCommand::Inspect {
                payload: "Check commit 154f.".into(),
                require: vec![cap("github.inspect")]
            }
        );
        let WireCommand::Inspect { require, .. } =
            command("x\n/inspect::wire --require=github.inspect,web.search,a-b_c.d1")
        else {
            panic!()
        };
        assert_eq!(
            require,
            [cap("github.inspect"), cap("web.search"), cap("a-b_c.d1")]
        );
    }

    #[test]
    fn t11_t14_status_and_trace_with_and_without_id() {
        assert_eq!(command("/status::wire"), WireCommand::Status { id: None });
        assert_eq!(
            command("/status::wire obj-42"),
            WireCommand::Status {
                id: Some(id("obj-42"))
            }
        );
        assert_eq!(command("/trace::wire"), WireCommand::Trace { id: None });
        assert_eq!(
            command("/trace::wire quest:tai-001.1"),
            WireCommand::Trace {
                id: Some(id("quest:tai-001.1"))
            }
        );
        assert_eq!(
            error("/status::wire a b"),
            "wire: status takes at most one <id>"
        );
    }

    #[test]
    fn t15_cancel_requires_an_id() {
        assert_eq!(error("/cancel::wire"), "wire: cancel requires <id>");
        assert_eq!(
            command("/cancel::wire obj-42"),
            WireCommand::Cancel { id: id("obj-42") }
        );
        assert_eq!(error("/cancel::wire a$b"), "wire: malformed id 'a$b'");
    }

    #[test]
    fn t16_malformed_commands_are_invalid_not_chat() {
        assert_eq!(
            error("/task::wire--dry-run"),
            "wire: malformed command '/task::wire--dry-run'"
        );
        assert_eq!(error("/::wire"), "wire: malformed command '/::wire'");
        assert_eq!(error("/a:b::wire"), "wire: malformed command '/a:b::wire'");
        assert_eq!(
            error("x\n/deploy::wire"),
            "wire: unknown command '/deploy::wire'"
        );
        assert_eq!(error("/help::wire"), "wire: /help::wire is reserved for V1");
    }

    #[test]
    fn t17_t18_unknown_and_unsupported_flags() {
        assert_eq!(
            error("x\n/task::wire --banana"),
            "wire: unknown flag '--banana'"
        );
        assert_eq!(error("x\n/task::wire -v"), "wire: unknown flag '-v'");
        assert_eq!(
            error("x\n/task::wire --review"),
            "wire: task does not accept '--review'"
        );
        assert_eq!(
            error("x\n/objective::wire --dry-run"),
            "wire: objective does not accept '--dry-run'"
        );
        assert_eq!(
            error("/status::wire --verbose"),
            "wire: status does not accept '--verbose'"
        );
        assert_eq!(
            error("x\n/objective::wire --review=yes"),
            "wire: flag '--review' takes no value"
        );
        assert_eq!(
            error("x\n/objective::wire --review --review"),
            "wire: duplicate flag '--review'"
        );
    }

    #[test]
    fn t19_t21_payload_is_required() {
        assert_eq!(
            error("/objective::wire --review"),
            "wire: objective requires payload"
        );
        assert_eq!(error("  \n\n/task::wire"), "wire: task requires payload");
        assert_eq!(
            error("/inspect::wire --require=github.inspect"),
            "wire: inspect requires payload"
        );
        assert_eq!(
            error("text\n/objective::wire stray"),
            "wire: objective takes no arguments; put the text above the command ('stray')"
        );
        assert_eq!(
            error("what is going on?\n/status::wire"),
            "wire: status takes no text; send it as its own message"
        );
    }

    #[test]
    fn t22_malformed_capabilities() {
        assert_eq!(
            error("x\n/inspect::wire --require=github..inspect"),
            "wire: malformed capability 'github..inspect'"
        );
        assert_eq!(
            error("x\n/inspect::wire --require=github.inspect,,web.search"),
            "wire: malformed capability list 'github.inspect,,web.search'"
        );
        assert_eq!(
            error("x\n/inspect::wire --require=GitHub.inspect"),
            "wire: malformed capability 'GitHub.inspect'"
        );
        assert_eq!(
            error("x\n/inspect::wire"),
            "wire: inspect requires at least one capability"
        );
        assert_eq!(
            error("x\n/inspect::wire --require="),
            "wire: inspect requires at least one capability"
        );
        assert_eq!(
            error("x\n/inspect::wire --require"),
            "wire: --require needs a value"
        );
        assert_eq!(
            error("x\n/inspect::wire --require=a.b,a.b"),
            "wire: duplicate capability 'a.b'"
        );
    }

    #[test]
    fn t23_newlines_and_unicode_survive() {
        let text = "prvi red\ndrugi red   \n\n\ttreći 🐙 red\n/task::wire";
        let WireCommand::Task { payload, .. } = command(text) else {
            panic!()
        };
        assert_eq!(payload, "prvi red\ndrugi red   \n\n\ttreći 🐙 red");
        // Leading blank lines and the separator before the command go.
        let WireCommand::Task { payload, .. } = command("\n\n  body\n\n\n/task::wire") else {
            panic!()
        };
        assert_eq!(payload, "  body");
    }

    #[test]
    fn t24_only_canonical_syntax_activates() {
        assert_eq!(
            error("x\n/Task::wire"),
            "wire: unknown command '/Task::wire'"
        );
        assert_eq!(
            error("x\n/TASK::wire"),
            "wire: unknown command '/TASK::wire'"
        );
        assert_eq!(
            error("x\n/task::Wire"),
            "wire: malformed command '/task::Wire'"
        );
        assert_eq!(parse("x\n/task:wire"), Parsed::Ordinary);
        assert_eq!(parse("x\ntask::wire"), Parsed::Ordinary);
        assert_eq!(
            parse("x\n /task::wire"),
            command_result("x", false),
            "indentation is not syntax"
        );
    }

    fn command_result(payload: &str, dry_run: bool) -> Parsed {
        Parsed::Command(WireCommand::Task {
            payload: payload.into(),
            dry_run,
            recall: false,
        })
    }

    #[test]
    fn t25_syntax_never_grants_authority() {
        let all = [
            command("g\n/objective::wire --review --verbose"),
            command("t\n/task::wire"),
            command("t\n/task::wire --dry-run"),
            command("i\n/inspect::wire --require=github.inspect"),
            command("/status::wire"),
            command("/trace::wire x"),
            command("/cancel::wire x"),
        ];
        for c in &all {
            // The strongest outcome syntax can describe is "subject to
            // authority"; there is no granting variant to return.
            assert!(matches!(
                c.side_effects(),
                SideEffects::Never | SideEffects::SubjectToAuthority
            ));
        }
        let never = |c: &WireCommand| c.side_effects() == SideEffects::Never;
        assert!(never(&all[2]), "dry run");
        assert!(never(&all[3]), "inspect is read-only");
        assert!(never(&all[4]) && never(&all[5]) && never(&all[6]));
        assert_eq!(all[0].side_effects(), SideEffects::SubjectToAuthority);
    }

    #[test]
    fn carriage_returns_do_not_break_activation() {
        assert_eq!(
            command("goal\r\n/objective::wire --review\r\n"),
            WireCommand::Objective {
                payload: "goal".into(),
                review: true,
                verbose: false
            }
        );
    }

    #[test]
    fn never_panics_on_arbitrary_utf8() {
        let pieces = [
            "",
            " ",
            "\n",
            "\r\n",
            "\t",
            "/",
            "::",
            "wire",
            "::wire",
            "/task::wire",
            "/objective::wire",
            "/cancel::wire",
            "--",
            "--review",
            "--require=",
            ",",
            ".",
            "=",
            "-",
            "🐙",
            "ž",
            "\u{0}",
            "\u{200b}",
            "a",
            "github.inspect",
            "x$y",
        ];
        // Deterministic pseudo-random concatenations.
        let mut seed: u64 = 0x5eed;
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..(seed % 9) {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                text.push_str(pieces[(seed >> 33) as usize % pieces.len()]);
            }
            seed = seed.wrapping_add(7);
            // Whatever parses keeps its invariants: no empty payloads.
            if let Parsed::Command(
                WireCommand::Objective { payload, .. }
                | WireCommand::Task { payload, .. }
                | WireCommand::Inspect { payload, .. },
            ) = parse(&text)
            {
                assert!(!payload.trim().is_empty());
            }
        }
    }
}
