//! `deaddrop-klodik [--once]`: run Klodik as a Deaddrop peer in the
//! foreground. Polls, answers new verified messages, waits, repeats.
//!
//! Environment:
//!   DEADDROP_HOME          Klodik's own node home (required)
//!   KLODIK_ALLOW           comma-separated node ids allowed to talk to
//!                          Klodik (default iva:local:deaddrop); each must
//!                          also be a trusted peer
//!   KLODIK_CLAUDE          model CLI to run (default `claude`)
//!   KLODIK_MODEL           model alias or name passed to the CLI (optional)
//!   KLODIK_TIMEOUT_SECS    per-call timeout (default 120)
//!   KLODIK_POLL_MS    idle poll (default 500); 100 ms right after work
//!                          (KLODIK_POLL_SECS is still read)
//!
//! State lives in `$DEADDROP_HOME/klodik/state.json`; completed exchanges,
//! the bounded conversational context, in `$DEADDROP_HOME/klodik/
//! transcripts.json`. The model runs in an empty directory outside the node
//! home and keeps no session of its own.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use deaddrop_klodik::context::{Conversational, Transcripts};
use deaddrop_klodik::model::ClaudeCli;
use deaddrop_klodik::{Klodik, State};
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
                    "unknown argument {other:?}; usage: deaddrop-klodik [--once]"
                ));
            }
        }
    }
    let home = PathBuf::from(env("DEADDROP_HOME").ok_or("DEADDROP_HOME is required")?);
    let allowed = env("KLODIK_ALLOW")
        .unwrap_or_else(|| "iva:local:deaddrop".into())
        .split(',')
        .map(|id| NodeId::parse(id.trim()).map_err(|e| format!("KLODIK_ALLOW {id:?}: {e}")))
        .collect::<Result<Vec<_>, _>>()?;
    // Soon after work, backing off to the idle poll while quiet.
    let mut pacer = deaddrop_klodik::Pacer::new(
        Duration::from_millis(100),
        deaddrop_klodik::idle_poll("KLODIK")?,
    );

    let shell = Shell::open(&home).map_err(|e| format!("{}: {e}", home.display()))?;
    let me = shell.identity().node;
    let trusted: Vec<NodeId> = shell
        .peers()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for id in &allowed {
        if !trusted.contains(id) {
            eprintln!(
                "klodik: warning: {id} is allowed but not a trusted peer; it cannot reach Klodik"
            );
        }
    }
    let model = ClaudeCli {
        program: env("KLODIK_CLAUDE")
            .unwrap_or_else(|| "claude".into())
            .into(),
        model: env("KLODIK_MODEL"),
        timeout: secs("KLODIK_TIMEOUT_SECS", 120)?,
        workdir: std::env::temp_dir().join(format!("deaddrop-klodik-{}", std::process::id())),
    };
    let state = State::load(&home.join("klodik").join("state.json"))?;
    let model = Conversational {
        model,
        transcripts: Transcripts::load(&home.join("klodik").join("transcripts.json"))?,
        own: me.to_string(),
    };
    let mut klodik = Klodik {
        shell,
        model,
        state,
        allowed,
        rooms: deaddrop_room::load_rooms(&home)?,
        may_ask: deaddrop_klodik::node_list("KLODIK_MAY_ASK")?,
    };
    eprintln!(
        "klodik: {me} listening for {}",
        klodik
            .allowed
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );

    loop {
        let worked = match klodik.step() {
            Ok(step) => {
                if let Some(error) = &step.sync_error {
                    eprintln!("klodik: sync failed, handling local messages only: {error}");
                }
                for reply in &step.replied {
                    eprintln!("klodik: replied {reply}");
                }
                for (id, to) in &step.asked {
                    eprintln!("klodik: asked {to} in {id}");
                }
                for (id, why) in &step.ask_refused {
                    eprintln!("klodik: no ask for {id}: {why}");
                }
                for id in &step.refused {
                    eprintln!("klodik: refused {id}: sender not allowed");
                }
                for (id, error) in &step.failed {
                    eprintln!("klodik: {id}: {error}");
                }
                step.handled > 0
            }
            Err(error) => {
                eprintln!("klodik: {error}");
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
            eprintln!("deaddrop-klodik: {error}");
            ExitCode::FAILURE
        }
    }
}
