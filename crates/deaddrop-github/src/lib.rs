//! The GitHub peer: a read-only Deaddrop front door to the existing Agent
//! Wire `github-inspector` worker.
//!
//! It answers exactly one question, through the worker's one operation
//! (`github.inspect` · `commit_modifies_path`):
//!
//!   does <owner/repo>@<40-char sha> modify <path>?
//!
//! The answer is the worker's claim and evidence, never a model's guess.
//! Anything else gets a truthful "not supported" reply. The worker reads a
//! commit's file list through `gh api` (a GET); this adapter exposes no
//! other GitHub operation. A chat request is not an Agent Wire quest, and
//! the reply is the worker's report, not a completion verdict.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use deaddrop_klodik::Answer;
use deaddrop_klodik::model::{ModelError, cut};
use deaddrop_protocol::EnvelopeV0;
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "github.inspect";
pub const OPERATION: &str = "commit_modifies_path";

/// The worker's request, exactly as `github-inspector` reads it on stdin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InspectRequest {
    pub operation: &'static str,
    pub repository: String,
    pub commit: String,
    pub path: String,
}

/// Words asking for something this peer must never do.
const WRITES: [&str; 16] = [
    "merge", "push", "edit", "write", "create", "delete", "close", "reopen", "comment", "approve",
    "revert", "rebase", "label", "assign", "tag", "release",
];

/// Words that introduce the path being asked about.
const PATH_WORDS: [&str; 8] = [
    "modify", "modifies", "modified", "touch", "touches", "touched", "path", "change",
];

pub fn help(reason: &str) -> String {
    format!(
        "Not supported: {reason}.\n\
         I'm the GitHub peer, read-only. I answer one question, via \
         {CAPABILITY} · {OPERATION}:\n  \
         does <owner/repo>@<40-char sha> modify <path>?\n\
         I can't merge, push, commit, edit, or change issues or PRs."
    )
}

fn clean(token: &str) -> &str {
    token
        .trim_matches(|c| matches!(c, '`' | '"' | '\'' | '(' | ')'))
        .trim_end_matches(['?', '!', ',', ';', ':'])
        .trim_end_matches('.')
}

fn is_sha(token: &str) -> bool {
    token.len() == 40 && token.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_repo(token: &str) -> bool {
    let mut parts = token.split('/');
    let ok = |s: Option<&str>| {
        s.is_some_and(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Read one chat message as an inspect request, or say why it is not one.
/// Strict on purpose: a message that does not plainly ask the one supported
/// question is not guessed at.
pub fn parse(body: &str, default_repo: Option<&str>) -> Result<InspectRequest, String> {
    let tokens: Vec<&str> = body.split_whitespace().map(clean).collect();
    let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    if let Some(word) = lower.iter().find(|t| WRITES.contains(&t.as_str())) {
        return Err(help(&format!("\"{word}\" would change GitHub")));
    }

    let mut repository = None;
    let mut commit = None;
    for token in &tokens {
        if let Some((repo, sha)) = token.split_once('@')
            && is_repo(repo)
            && is_sha(sha)
        {
            repository.get_or_insert_with(|| repo.to_owned());
            commit.get_or_insert_with(|| sha.to_lowercase());
        } else if is_sha(token) {
            commit.get_or_insert_with(|| token.to_lowercase());
        }
    }
    let after = |words: &[&str]| {
        lower
            .iter()
            .position(|t| words.contains(&t.as_str()))
            .and_then(|i| tokens.get(i + 1))
            .map(|t| (*t).to_owned())
    };
    if repository.is_none() {
        repository = after(&["in", "repo", "repository"]).filter(|r| is_repo(r));
    }
    let repository = repository.or_else(|| default_repo.map(str::to_owned));
    let path = after(&PATH_WORDS).filter(|p| !p.is_empty());

    match (repository, commit, path) {
        (Some(repository), Some(commit), Some(path)) => Ok(InspectRequest {
            operation: OPERATION,
            repository,
            commit,
            path,
        }),
        (_, None, _) => Err(help("no full 40-character commit sha")),
        (None, _, _) => Err(help("no repository (write owner/repo@sha)")),
        (_, _, None) => Err(help("no path (write: modify <path>)")),
    }
}

#[derive(Deserialize)]
struct Report {
    claim: Claim,
    evidence: Evidence,
}

#[derive(Deserialize)]
struct Claim {
    commit_modifies_path: bool,
}

#[derive(Deserialize)]
struct Evidence {
    repository: String,
    commit: String,
    path: String,
}

#[derive(Deserialize)]
struct Failure {
    status: String,
    error: FailureDetail,
}

#[derive(Deserialize)]
struct FailureDetail {
    kind: String,
    message: String,
}

/// The reply for a worker report: its claim and evidence, as they came.
pub fn render(stdout: &[u8]) -> Result<String, ModelError> {
    let report: Report =
        serde_json::from_slice(stdout).map_err(|e| ModelError::Malformed(e.to_string()))?;
    let verdict = if report.claim.commit_modifies_path {
        "yes, it modifies"
    } else {
        "no, it does not modify"
    };
    let e = report.evidence;
    Ok(format!(
        "{verdict} {}\n{}@{}\nsource: {CAPABILITY} · {OPERATION} (github-inspector, read-only)",
        e.path, e.repository, e.commit
    ))
}

/// The reply for a worker that declined (exit 2) or could not resolve
/// (exit 3): its own status and reason, said plainly. Not a success.
pub fn render_failure(stderr: &[u8]) -> Result<String, ModelError> {
    let failure: Failure =
        serde_json::from_slice(stderr).map_err(|e| ModelError::Malformed(e.to_string()))?;
    Ok(format!(
        "github-inspector {}: {} ({})\nNo answer to the question; nothing was changed.",
        failure.status,
        cut(&failure.error.message, 300),
        failure.error.kind
    ))
}

/// The existing `github-inspector` binary, run once per request.
#[derive(Debug, Clone)]
pub struct Inspector {
    pub program: PathBuf,
    pub timeout: Duration,
    /// Used when a message names no repository.
    pub default_repo: Option<String>,
}

impl Inspector {
    fn run(&self, request: &InspectRequest) -> Result<String, ModelError> {
        let input =
            serde_json::to_vec(request).map_err(|e| ModelError::Malformed(e.to_string()))?;
        let mut child = Command::new(&self.program)
            .env_remove("DEADDROP_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| ModelError::Spawn(format!("{}: {e}", self.program.display())))?;
        let mut stdin = child.stdin.take().expect("piped");
        let writer = std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
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
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ModelError::TimedOut(self.timeout));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let _ = writer.join();
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        match status.code() {
            Some(0) => render(&stdout),
            // The worker's own documented verdicts: rejected / unresolved.
            Some(2 | 3) => render_failure(&stderr),
            code => Err(ModelError::Failed {
                code,
                stderr: cut(String::from_utf8_lossy(&stderr).trim(), 300),
            }),
        }
    }
}

impl Answer for Inspector {
    fn answer(&self, message: &EnvelopeV0) -> Result<String, ModelError> {
        match parse(message.body(), self.default_repo.as_deref()) {
            Ok(request) => self.run(&request),
            // A truthful refusal is a reply; the worker is not called.
            Err(help) => Ok(help),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";

    #[test]
    fn a_structured_handoff_is_the_same_question() {
        // Research's request, as the room carries it.
        let handoff = format!(
            "request:: github.inspect\n→ @github\nrepo:: Tesliana-code/deaddrop\ncommit:: {SHA}\npath:: crates/deaddrop-tui/src/ui.rs"
        );
        let r = parse(&handoff, None).unwrap();
        assert_eq!(
            (r.repository.as_str(), r.commit.as_str(), r.path.as_str()),
            (
                "Tesliana-code/deaddrop",
                SHA,
                "crates/deaddrop-tui/src/ui.rs"
            )
        );
    }

    #[test]
    fn the_supported_question_maps_to_the_worker_request() {
        let r = parse(
            &format!("does Tesliana-code/deaddrop@{SHA} modify crates/deaddrop-tui/src/ui.rs?"),
            None,
        )
        .unwrap();
        assert_eq!(r.operation, "commit_modifies_path");
        assert_eq!(r.repository, "Tesliana-code/deaddrop");
        assert_eq!(r.commit, SHA);
        assert_eq!(r.path, "crates/deaddrop-tui/src/ui.rs");
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "operation": "commit_modifies_path",
                "repository": "Tesliana-code/deaddrop",
                "commit": SHA,
                "path": "crates/deaddrop-tui/src/ui.rs",
            }),
            "exactly the worker's input schema"
        );
    }

    #[test]
    fn other_phrasings_and_the_default_repo() {
        let r = parse(
            &format!("inspect commit {SHA} for path `README.md` in a/b"),
            None,
        )
        .unwrap();
        assert_eq!(
            (r.repository.as_str(), r.path.as_str()),
            ("a/b", "README.md")
        );
        let r = parse(
            &format!("does commit {SHA} touch backend/src/foo.rs?"),
            Some("o/r"),
        )
        .unwrap();
        assert_eq!(
            (r.repository.as_str(), r.path.as_str()),
            ("o/r", "backend/src/foo.rs")
        );
        let upper = SHA.to_uppercase();
        assert_eq!(
            parse(&format!("does o/r@{upper} modify x"), None)
                .unwrap()
                .commit,
            SHA
        );
    }

    #[test]
    fn anything_else_is_refused_truthfully() {
        for (body, reason) in [
            ("hello github", "sha"),
            ("does commit 154f992 modify ui.rs?", "sha"),
            (
                &format!("does commit {SHA} modify ui.rs?") as &str,
                "repository",
            ),
            (&format!("look at o/r@{SHA}"), "path"),
            (
                &format!("merge o/r@{SHA} please, it touches x"),
                "would change GitHub",
            ),
            (
                &format!("push o/r@{SHA} and modify x"),
                "would change GitHub",
            ),
            ("open a PR and comment", "would change GitHub"),
        ] {
            let why = parse(body, None).unwrap_err();
            assert!(why.starts_with("Not supported"), "{body}");
            assert!(why.contains(reason), "{body}: {why}");
            assert!(why.contains("read-only"));
        }
    }

    #[test]
    fn replies_carry_the_worker_claim_and_evidence() {
        let yes = br#"{"claim":{"commit_modifies_path":true},"evidence":{"repository":"o/r","commit":"abc","path":"x/y.rs"}}"#;
        let reply = render(yes).unwrap();
        assert!(reply.starts_with("yes, it modifies x/y.rs"));
        assert!(reply.contains("o/r@abc"));
        assert!(reply.contains("github.inspect · commit_modifies_path"));
        let no = br#"{"claim":{"commit_modifies_path":false},"evidence":{"repository":"o/r","commit":"abc","path":"z"}}"#;
        assert!(render(no).unwrap().starts_with("no, it does not modify z"));
        assert!(matches!(render(b"maybe"), Err(ModelError::Malformed(_))));
    }

    #[test]
    fn worker_refusals_are_reported_as_refusals() {
        let stderr = br#"{"status":"unresolved","error":{"kind":"evidence_unavailable","message":"HTTP 404"}}"#;
        let reply = render_failure(stderr).unwrap();
        assert!(reply.starts_with("github-inspector unresolved: HTTP 404"));
        assert!(reply.contains("No answer"));
        assert!(matches!(
            render_failure(b"boom"),
            Err(ModelError::Malformed(_))
        ));
    }
}
