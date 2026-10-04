//! `deaddrop-github [--once]`: run the read-only GitHub peer in the
//! foreground, answering through the existing `github-inspector` worker.
//!
//! Environment:
//!   DEADDROP_HOME              the GitHub peer's own node home (required)
//!   GITHUB_INSPECTOR           path to the github-inspector binary (required)
//!   GITHUB_PEER_ALLOW          comma-separated node ids allowed to ask
//!                              (default iva:local:deaddrop)
//!   GITHUB_PEER_DEFAULT_REPO   owner/repo used when a message names none
//!   GITHUB_PEER_TIMEOUT_SECS   per-request timeout (default 60)
//!   GITHUB_PEER_POLL_MS idle poll (default 500); 100 ms right after work
//!                          (GITHUB_PEER_POLL_SECS is still read)
//!
//! State lives in `$DEADDROP_HOME/github-peer/state.json`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use deaddrop_github::Inspector;
use deaddrop_klodik::{Peer, State};
use deaddrop_protocol::NodeId;
use deaddrop_shell::Shell;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn secs(name: &str, default: u64) -> Result<Duration, String> {
    match env(name) {
        None => Ok(Duration::from_secs(default)),
        Some(v) => v
            .parse()
            .map(Duration::from_secs)
            .map_err(|_| format!("{name} must be whole seconds, got {v:?}")),
    }
}

fn run() -> Result<(), String> {
    let mut once = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--once" => once = true,
            other => {
                return Err(format!(
                    "unknown argument {other:?}; usage: deaddrop-github [--once]"
                ));
            }
        }
    }
    let home = PathBuf::from(env("DEADDROP_HOME").ok_or("DEADDROP_HOME is required")?);
    let program = PathBuf::from(env("GITHUB_INSPECTOR").ok_or("GITHUB_INSPECTOR is required")?);
    if !program.is_file() {
        return Err(format!(
            "GITHUB_INSPECTOR {} is not a file",
            program.display()
        ));
    }
    let allowed = env("GITHUB_PEER_ALLOW")
        .unwrap_or_else(|| "iva:local:deaddrop".into())
        .split(',')
        .map(|id| NodeId::parse(id.trim()).map_err(|e| format!("GITHUB_PEER_ALLOW {id:?}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    // Soon after work, backing off to the idle poll while quiet.
    let mut pacer = deaddrop_klodik::Pacer::new(
        Duration::from_millis(100),
        deaddrop_klodik::idle_poll("GITHUB_PEER")?,
    );

    let shell = Shell::open(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let me = shell.identity().node;
    let inspector = Inspector {
        program,
        timeout: secs("GITHUB_PEER_TIMEOUT_SECS", 60)?,
        default_repo: env("GITHUB_PEER_DEFAULT_REPO"),
    };
    let state = State::load(&home.join("github-peer").join("state.json"))?;
    let mut peer = Peer {
        shell,
        model: inspector,
        state,
        allowed,
        rooms: deaddrop_room::load_rooms(&home)?,
        may_ask: deaddrop_klodik::node_list("GITHUB_PEER_MAY_ASK")?,
    };
    eprintln!(
        "github: {me} answering github.inspect · commit_modifies_path for {}",
        peer.allowed
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );

    loop {
        let worked = match peer.step() {
            Ok(step) => {
                if let Some(error) = &step.sync_error {
                    eprintln!("github: sync failed, handling local messages only: {error}");
                }
                for reply in &step.replied {
                    eprintln!("github: replied {reply}");
                }
                for (id, to) in &step.asked {
                    eprintln!("github: asked {to} in {id}");
                }
                for (id, why) in &step.ask_refused {
                    eprintln!("github: no ask for {id}: {why}");
                }
                for id in &step.refused {
                    eprintln!("github: refused {id}: sender not allowed");
                }
                for (id, error) in &step.failed {
                    eprintln!("github: {id}: {error}");
                }
                step.handled > 0
            }
            Err(error) => {
                eprintln!("github: {error}");
                false
            }
        };
        if once {
            return Ok(());
        }
        std::thread::sleep(pacer.next(worked));
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("deaddrop-github: {error}");
            ExitCode::FAILURE
        }
    }
}
