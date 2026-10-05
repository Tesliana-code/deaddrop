//! /task::wire V0: from a task in a room to governed work.
//!
//! ```text
//! WireCommand::Task (deterministic parser, deaddrop-wire-command)
//!   → planner::Planner      proposes capability steps (a model, no tools)
//!   → plan::validate        decides legality: capabilities, DAG, bounds,
//!                           side effects; registry::resolve decides trust,
//!                           reachability, policy and authority per worker
//!   → run::TaskRun          one Agent Wire quest per step; independent
//!                           steps assigned together, dependents after
//!   → room requests / reports, evaluated by Agent Wire, completed once
//! ```
//!
//! Agent Wire (`agent-wire-coordinate`) is the completion authority. Nothing
//! here can grant a worker more than its capability's read-only or
//! report-only mode.

pub mod journal;
pub mod memory;
pub mod plan;
pub mod planner;
pub mod registry;
pub mod run;

/// `T-` and six hex digits, from a fresh message id.
pub fn task_id(fresh: &str) -> String {
    let hex: String = fresh
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .take(6)
        .collect();
    format!("T-{hex}")
}

/// Whether `id` is a task id: `T-` and six lowercase hex digits.
pub fn is_task_id(id: &str) -> bool {
    id.strip_prefix("T-").is_some_and(|hex| {
        hex.len() == 6
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
}
