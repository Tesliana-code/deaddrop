//! Plan a task with the real planner and validate it, printing only the
//! validated plan (or why it is invalid) and how long planning took.
//! Runs no worker.
//!
//!   cargo run -q -p deaddrop-task --example task-plan -- "<task>" [model]

use std::time::{Duration, Instant};

use deaddrop_task::plan::validate;
use deaddrop_task::planner::{ClaudePlanner, Planner};
use deaddrop_task::registry::{Context, REGISTRY};

fn main() {
    let mut args = std::env::args().skip(1);
    let task = args.next().expect("task text");
    let planner = ClaudePlanner {
        program: "claude".into(),
        model: args.next(),
        timeout: Duration::from_secs(90),
        workdir: std::env::temp_dir().join(format!("deaddrop-task-plan-{}", std::process::id())),
    };
    let peers: Vec<String> = REGISTRY.iter().map(|c| c.peer.to_owned()).collect();
    let ctx = Context {
        trusted: peers.clone(),
        members: peers.clone(),
        may_ask: peers,
    };
    let started = Instant::now();
    let proposed = planner.propose(&task);
    let ms = started.elapsed().as_millis();
    match proposed.and_then(|p| validate(&task, &p, &ctx)) {
        Ok(plan) => {
            let steps: Vec<String> = plan
                .steps
                .iter()
                .map(|s| format!("{}:{}<-[{}]", s.id, s.capability, s.depends_on.join(",")))
                .collect();
            println!("{ms} ms  valid  {}", steps.join("  "));
        }
        Err(why) => println!("{ms} ms  invalid  {why}"),
    }
}
