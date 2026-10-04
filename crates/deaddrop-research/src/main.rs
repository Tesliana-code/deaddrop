//! `deaddrop-research [--once]`: run the Research peer in the foreground.
//! Polls, answers new verified questions from web search, waits, repeats.
//!
//! Environment:
//!   DEADDROP_HOME           the Research peer's own node home (required)
//!   RESEARCH_ALLOW          comma-separated node ids allowed to ask
//!                           (default iva:local:deaddrop)
//!   RESEARCH_CLAUDE         model CLI to run (default `claude`)
//!   RESEARCH_MODEL          model alias or name passed to the CLI (optional)
//!   RESEARCH_TIMEOUT_SECS   per-question timeout (default 120, at most 120)
//!   RESEARCH_PROFILE        `fast` (default: one search, at most 3 sources)
//!                           or `deep`; an operator setting, not message syntax
//!   RESEARCH_POLL_MS  idle poll (default 500); 100 ms right after work
//!                          (RESEARCH_POLL_SECS is still read)
//!
//! State lives in `$DEADDROP_HOME/research/state.json`. The model runs in an
//! empty directory outside the node home.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use deaddrop_klodik::{Peer, State};
use deaddrop_protocol::NodeId;
use deaddrop_research::ResearchCli;
use deaddrop_shell::Shell;

const MAX_TIMEOUT_SECS: u64 = 120;

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
                    "unknown argument {other:?}; usage: deaddrop-research [--once]"
                ));
            }
        }
    }
    let home = PathBuf::from(env("DEADDROP_HOME").ok_or("DEADDROP_HOME is required")?);
    let allowed = env("RESEARCH_ALLOW")
        .unwrap_or_else(|| "iva:local:deaddrop".into())
        .split(',')
        .map(|id| NodeId::parse(id.trim()).map_err(|e| format!("RESEARCH_ALLOW {id:?}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    let timeout = secs("RESEARCH_TIMEOUT_SECS", MAX_TIMEOUT_SECS)?;
    if timeout > Duration::from_secs(MAX_TIMEOUT_SECS) {
        return Err(format!(
            "RESEARCH_TIMEOUT_SECS is at most {MAX_TIMEOUT_SECS}"
        ));
    }
    let profile = match env("RESEARCH_PROFILE").as_deref() {
        None | Some("fast") => deaddrop_research::Profile::Fast,
        Some("deep") => deaddrop_research::Profile::Deep,
        Some(other) => return Err(format!("RESEARCH_PROFILE {other:?}: fast or deep")),
    };
    // Soon after work, backing off to the idle poll while quiet.
    let mut pacer = deaddrop_klodik::Pacer::new(
        Duration::from_millis(100),
        deaddrop_klodik::idle_poll("RESEARCH")?,
    );

    let shell = Shell::open(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let me = shell.identity().node;
    let runtime = ResearchCli {
        program: env("RESEARCH_CLAUDE")
            .unwrap_or_else(|| "claude".into())
            .into(),
        model: env("RESEARCH_MODEL"),
        timeout,
        workdir: std::env::temp_dir().join(format!("deaddrop-research-{}", std::process::id())),
        profile,
    };
    let state = State::load(&home.join("research").join("state.json"))?;
    let mut peer = Peer {
        shell,
        model: runtime,
        state,
        allowed,
        rooms: deaddrop_room::load_rooms(&home)?,
        may_ask: deaddrop_klodik::node_list("RESEARCH_MAY_ASK")?,
    };
    eprintln!(
        "research: {me} answering with web search ({}) for {}",
        profile.name(),
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
                    eprintln!("research: sync failed, handling local messages only: {error}");
                }
                for reply in &step.replied {
                    eprintln!("research: replied {reply}");
                }
                for (id, to) in &step.asked {
                    eprintln!("research: asked {to} in {id}");
                }
                for (id, why) in &step.ask_refused {
                    eprintln!("research: no ask for {id}: {why}");
                }
                for id in &step.refused {
                    eprintln!("research: refused {id}: sender not allowed");
                }
                for (id, error) in &step.failed {
                    eprintln!("research: {id}: {error}");
                }
                step.handled > 0
            }
            Err(error) => {
                eprintln!("research: {error}");
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
            eprintln!("deaddrop-research: {error}");
            ExitCode::FAILURE
        }
    }
}
