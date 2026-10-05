//! `deaddrop-memory --home <dir> [filters]`: structured, read-only queries
//! over a node's episodic task memory. Every episode comes with the refs
//! that explain it. Reads journals, (re)derives episode files; touches
//! nothing else and reaches no network.
//!
//!   --task T-…  --status complete|failed  --capability <id>
//!   --worker <node id>  --room <name>  --scope task/…|room/…|peer/…|project/…
//!   --recent N  --json

use std::path::PathBuf;
use std::process::ExitCode;

use deaddrop_task::memory::{Query, query};

const USAGE: &str = "usage: deaddrop-memory --home <dir> [--task T-…] [--status complete|failed] [--capability <id>] [--worker <node id>] [--room <name>] [--scope <scope>] [--recent N] [--json]";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut home = std::env::var_os("DEADDROP_HOME").map(PathBuf::from);
    let mut q = Query::default();
    let mut json = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let set = match arg.as_str() {
            "--home" => value().map(|v| home = Some(v.into())),
            "--task" => value().map(|v| q.task = Some(v)),
            "--status" => value().map(|v| q.status = Some(v)),
            "--capability" => value().map(|v| q.capability = Some(v)),
            "--worker" => value().map(|v| q.worker = Some(v)),
            "--room" => value().map(|v| q.room = Some(v)),
            "--scope" => value().map(|v| q.scope = Some(v)),
            "--recent" => value().and_then(|v| {
                v.parse()
                    .map(|n| q.limit = Some(n))
                    .map_err(|_| "--recent takes a number".to_owned())
            }),
            "--json" => {
                json = true;
                Ok(())
            }
            other => Err(format!("unknown argument {other:?}")),
        };
        if let Err(e) = set {
            eprintln!("deaddrop-memory: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(home) = home else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let (hits, recall) = match query(&home, &q) {
        Ok(found) => found,
        Err(e) => {
            eprintln!("deaddrop-memory: {e}");
            return ExitCode::FAILURE;
        }
    };
    for (task, why) in &recall.errors {
        eprintln!("deaddrop-memory: journal {task}: {why} · not remembered");
    }
    for task in &recall.orphans {
        eprintln!("deaddrop-memory: episode {task} has no journal · ignored");
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&hits).expect("episodes serialize")
        );
        return ExitCode::SUCCESS;
    }
    if hits.is_empty() {
        println!("no episodes");
    }
    for e in &hits {
        println!("{} · {}", e.episode_id, e.status);
        println!("  objective:: {}", e.objective.lines().next().unwrap_or(""));
        println!("  scope:: {} · {}", e.scope.task, e.scope.room);
        println!("  capabilities:: {}", e.capabilities.join(", "));
        for s in &e.steps {
            let report = s.report.as_deref().unwrap_or("-");
            let seq = s
                .complete_seq
                .map(|n| format!(" · wire COMPLETE #{n}"))
                .unwrap_or_default();
            println!(
                "  step:: {} {} → {} · {} · report {report}{seq}",
                s.id, s.capability, s.worker, s.outcome
            );
        }
        if let Some(why) = &e.failure_reason {
            println!("  reason:: {why}");
        }
        if let Some(r) = &e.final_report {
            println!("  final_report:: {r}");
        }
        for r in e.refs() {
            println!("  provenance:: {r}");
        }
    }
    ExitCode::SUCCESS
}
