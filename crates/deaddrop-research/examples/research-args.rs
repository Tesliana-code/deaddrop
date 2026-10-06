//! Prints Research's exact CLI invocation as JSON, for measurement
//! harnesses (dev/research-latency.py): `{"args": [...], "stdin": "..."}`.
//!
//!   cargo run -q -p deaddrop-research --example research-args -- <from> <body>

use std::path::PathBuf;
use std::time::Duration;

use deaddrop_research::{Profile, ResearchCli, question};

fn main() {
    let mut argv = std::env::args().skip(1);
    let (from, body) = (
        argv.next().unwrap_or_default(),
        argv.next().unwrap_or_default(),
    );
    let cli = ResearchCli {
        program: PathBuf::from("claude"),
        model: std::env::var("RESEARCH_MODEL")
            .ok()
            .filter(|m| !m.is_empty()),
        timeout: Duration::from_secs(120),
        workdir: PathBuf::new(),
        profile: match std::env::var("RESEARCH_PROFILE").as_deref() {
            Ok("fast") => Profile::Fast,
            _ => Profile::Deep,
        },
    };
    let mut args = cli.args();
    // Extra CLI flags under test, e.g. `--effort low`.
    if let Ok(extra) = std::env::var("RESEARCH_EXTRA_ARGS") {
        args.extend(extra.split_whitespace().map(str::to_owned));
    }
    let out = serde_json::json!({ "args": args, "stdin": question(&from, &body) });
    println!("{out}");
}
