//! Plans: what the planner proposes, and the deterministic validator that
//! decides whether it is legal. The planner proposes capabilities, never
//! peers; trust, policy and authority are decided here, not by a model.

use serde::Deserialize;

use crate::registry::{self, Context, GITHUB_INSPECT, Mode, SYNTHESIZE, WEB_SEARCH};

pub const MAX_STEPS: usize = 5;
pub const MAX_WORKERS: usize = 3;
/// Longest objective a step may carry.
pub const MAX_OBJECTIVE_CHARS: usize = 1000;

/// The planner's output shape (`--json-schema`); checked again here.
pub const SCHEMA: &str = r#"{"type":"object","properties":{"steps":{"type":"array","maxItems":5,"items":{"type":"object","properties":{"id":{"type":"string","pattern":"^[a-z][a-z0-9_]{0,31}$"},"capability":{"type":"string","enum":["web.search","github.inspect","synthesize"]},"objective":{"type":"string"},"depends_on":{"type":"array","items":{"type":"string"}}},"required":["id","capability","objective","depends_on"],"additionalProperties":false}}},"required":["steps"],"additionalProperties":false}"#;

/// A plan as proposed: data, not yet permission for anything.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposed {
    pub steps: Vec<ProposedStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedStep {
    pub id: String,
    pub capability: String,
    pub objective: String,
    pub depends_on: Vec<String>,
}

/// A validated step: its capability resolved to a permitted worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    pub capability: &'static str,
    pub worker: String,
    /// The authority the capability had when the plan was validated.
    pub mode: Mode,
    pub objective: String,
    pub depends_on: Vec<String>,
    /// What the worker is sent, without the task header. For
    /// `synthesize` it is built later, from the reports it depends on.
    pub request: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }

    /// Distinct workers, in plan order.
    pub fn workers(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.steps {
            if !out.contains(&s.worker.as_str()) {
                out.push(&s.worker);
            }
        }
        out
    }
}

/// Words that ask for a change in the world. V0 has no capability that
/// writes, so a task or step asking for one is refused, not reinterpreted.
const SIDE_EFFECTS: [&str; 10] = [
    "merge",
    "push",
    "delete",
    "deploy",
    "publish",
    "approve",
    "revert",
    "rebase",
    "install",
    "uninstall",
];

fn side_effect(text: &str) -> Option<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::to_ascii_lowercase)
        .find(|w| SIDE_EFFECTS.contains(&w.as_str()))
}

/// Parse the planner's structured output. Malformed is invalid, never
/// "close enough".
pub fn parse(json: &serde_json::Value) -> Result<Proposed, String> {
    serde_json::from_value(json.clone()).map_err(|e| format!("planner output: {e}"))
}

/// Decide whether `proposed` is a legal plan for `task` under `ctx`.
pub fn validate(task: &str, proposed: &Proposed, ctx: &Context) -> Result<Plan, String> {
    if let Some(word) = side_effect(task) {
        return Err(format!("side-effect request ({word:?}): V0 is read-only"));
    }
    let n = proposed.steps.len();
    if n == 0 {
        return Err("the planner found no plan the available capabilities can do".into());
    }
    if n > MAX_STEPS {
        return Err(format!("{n} steps: at most {MAX_STEPS}"));
    }
    let mut steps: Vec<Step> = Vec::new();
    for p in &proposed.steps {
        let ok_id = p.id.len() <= 32
            && p.id.starts_with(|c: char| c.is_ascii_lowercase())
            && p.id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !ok_id {
            return Err(format!("step id {:?} is not snake_case", p.id));
        }
        if steps.iter().any(|s| s.id == p.id) {
            return Err(format!("duplicate step id {:?}", p.id));
        }
        let objective = p.objective.trim();
        if objective.is_empty() || objective.chars().count() > MAX_OBJECTIVE_CHARS {
            return Err(format!("step {}: objective empty or too long", p.id));
        }
        if let Some(word) = side_effect(objective) {
            return Err(format!("step {}: side-effect request ({word:?})", p.id));
        }
        let c = registry::resolve(&p.capability, ctx).map_err(|e| format!("step {}: {e}", p.id))?;
        // Only synthesis consumes reports; any other dependency would only
        // serialize independent work.
        match (c.id, p.depends_on.is_empty()) {
            (SYNTHESIZE, true) => {
                return Err(format!(
                    "step {}: synthesize needs reports to depend on",
                    p.id
                ));
            }
            (WEB_SEARCH | GITHUB_INSPECT, false) => {
                return Err(format!(
                    "step {}: {} takes no dependencies (only synthesize consumes reports)",
                    p.id, c.id
                ));
            }
            _ => {}
        }
        let request = match c.id {
            GITHUB_INSPECT => {
                let r = deaddrop_github::parse(objective, None)
                    .map_err(|e| format!("step {}: not an inspect request: {}", p.id, first(&e)))?;
                format!(
                    "request:: {GITHUB_INSPECT}\nrepo:: {}\ncommit:: {}\npath:: {}",
                    r.repository, r.commit, r.path
                )
            }
            _ => objective.to_owned(),
        };
        steps.push(Step {
            id: p.id.clone(),
            capability: c.id,
            worker: c.peer.to_owned(),
            mode: c.mode,
            objective: objective.to_owned(),
            depends_on: p.depends_on.clone(),
            request,
        });
    }
    for s in &steps {
        for d in &s.depends_on {
            if d == &s.id {
                return Err(format!("step {} depends on itself", s.id));
            }
            if !steps.iter().any(|t| &t.id == d) {
                return Err(format!("step {} depends on missing step {d:?}", s.id));
            }
        }
    }
    acyclic(&steps)?;
    let plan = Plan { steps };
    let workers = plan.workers().len();
    if workers > MAX_WORKERS {
        return Err(format!("{workers} workers: at most {MAX_WORKERS}"));
    }
    Ok(plan)
}

fn first(text: &str) -> &str {
    text.lines().next().unwrap_or(text)
}

/// Kahn's algorithm: every step must become ready at some point.
pub(crate) fn acyclic(steps: &[Step]) -> Result<(), String> {
    let mut done: Vec<&str> = Vec::new();
    while done.len() < steps.len() {
        let ready: Vec<&str> = steps
            .iter()
            .filter(|s| !done.contains(&s.id.as_str()))
            .filter(|s| s.depends_on.iter().all(|d| done.contains(&d.as_str())))
            .map(|s| s.id.as_str())
            .collect();
        if ready.is_empty() {
            let stuck: Vec<&str> = steps
                .iter()
                .map(|s| s.id.as_str())
                .filter(|id| !done.contains(id))
                .collect();
            return Err(format!("dependency cycle among {}", stuck.join(", ")));
        }
        done.extend(ready);
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::registry::REGISTRY;

    pub const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";
    pub const TASK: &str = "Find the latest official Rust release, inspect whether commit 154f992cc3e88f51a0b6bdbf42998e94c3aeedaf in Tesliana-code/deaddrop modifies crates/deaddrop-tui/src/ui.rs, and summarize both findings.";

    pub fn ctx() -> Context {
        let peers: Vec<String> = REGISTRY.iter().map(|c| c.peer.to_owned()).collect();
        Context {
            trusted: peers.clone(),
            members: peers.clone(),
            may_ask: peers,
        }
    }

    pub fn step(id: &str, capability: &str, objective: &str, deps: &[&str]) -> ProposedStep {
        ProposedStep {
            id: id.into(),
            capability: capability.into(),
            objective: objective.into(),
            depends_on: deps.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    pub fn golden() -> Proposed {
        Proposed {
            steps: vec![
                step(
                    "research_release",
                    "web.search",
                    "What is the latest official stable Rust release?",
                    &[],
                ),
                step(
                    "inspect_commit",
                    "github.inspect",
                    &format!(
                        "does Tesliana-code/deaddrop@{SHA} modify crates/deaddrop-tui/src/ui.rs?"
                    ),
                    &[],
                ),
                step(
                    "synthesize",
                    "synthesize",
                    "Summarize both findings.",
                    &["research_release", "inspect_commit"],
                ),
            ],
        }
    }

    fn rejected(p: Proposed) -> String {
        validate(TASK, &p, &ctx()).unwrap_err()
    }

    #[test]
    fn the_golden_plan_is_valid_and_resolves_workers() {
        let plan = validate(TASK, &golden(), &ctx()).unwrap();
        assert_eq!(
            plan.workers(),
            [
                "research:agent:deaddrop",
                "github:agent:deaddrop",
                "klodik:agent:deaddrop"
            ]
        );
        let inspect = plan.step("inspect_commit").unwrap();
        assert_eq!(
            inspect.request,
            format!(
                "request:: github.inspect\nrepo:: Tesliana-code/deaddrop\ncommit:: {SHA}\npath:: crates/deaddrop-tui/src/ui.rs"
            )
        );
        // Independent steps are recognized as such.
        let independent: Vec<&str> = plan
            .steps
            .iter()
            .filter(|s| s.depends_on.is_empty())
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(independent, ["research_release", "inspect_commit"]);
    }

    #[test]
    fn schema_matches_the_registry_and_bounds() {
        let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
        assert_eq!(schema["properties"]["steps"]["maxItems"], MAX_STEPS);
        let ids: Vec<&str> = REGISTRY.iter().map(|c| c.id).collect();
        let enumerated: Vec<&str> =
            schema["properties"]["steps"]["items"]["properties"]["capability"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
        assert_eq!(enumerated, ids);
    }

    #[test]
    fn malformed_planner_output_is_invalid() {
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"steps": "three"}),
            serde_json::json!({"steps": [{"id": "a", "capability": "web.search", "objective": "x"}]}),
            serde_json::json!({"steps": [], "worker": "research"}),
            serde_json::json!({"steps": [{"id": "a", "capability": "web.search", "objective": "x", "depends_on": [], "peer": "github"}]}),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
        assert_eq!(
            parse(&serde_json::to_value(serde_json::json!({"steps": []})).unwrap())
                .unwrap()
                .steps
                .len(),
            0
        );
    }

    #[test]
    fn illegal_plans_are_rejected_with_a_reason() {
        let mut p = golden();
        p.steps[0].capability = "web.fetch".into();
        assert!(rejected(p).contains("unknown capability \"web.fetch\""));

        let mut p = golden();
        p.steps[1].id = "research_release".into();
        assert!(rejected(p).contains("duplicate step id"));

        let mut p = golden();
        p.steps[2].depends_on.push("nowhere".into());
        assert!(rejected(p).contains("missing step \"nowhere\""));

        let p = Proposed {
            steps: vec![
                step("a", "synthesize", "x", &["b"]),
                step("b", "synthesize", "y", &["a"]),
            ],
        };
        assert!(rejected(p).contains("dependency cycle among a, b"));

        let p = Proposed {
            steps: (0..6)
                .map(|i| step(&format!("s{i}"), "web.search", "q", &[]))
                .collect(),
        };
        assert!(rejected(p).contains("at most 5"));

        assert!(rejected(Proposed { steps: vec![] }).contains("no plan"));

        let mut p = golden();
        p.steps[1].depends_on.push("research_release".into());
        assert!(
            rejected(p).contains("takes no dependencies"),
            "no needless serializing"
        );

        let mut p = golden();
        p.steps[1].objective = "look at the deaddrop repo".into();
        assert!(rejected(p).contains("not an inspect request"));

        let mut p = golden();
        p.steps[0].objective = "find it and push a fix".into();
        assert!(rejected(p).contains("side-effect request (\"push\")"));
        assert!(
            validate("deploy the release", &golden(), &ctx())
                .unwrap_err()
                .contains("V0 is read-only")
        );
    }

    #[test]
    fn policy_and_trust_are_checked_per_worker() {
        let mut c = ctx();
        c.may_ask.retain(|p| !p.starts_with("klodik"));
        assert_eq!(
            validate(TASK, &golden(), &c).unwrap_err(),
            "step synthesize: synthesize: policy does not allow asking klodik"
        );
        let mut c = ctx();
        c.trusted.clear();
        assert!(
            validate(TASK, &golden(), &c)
                .unwrap_err()
                .contains("not a trusted peer")
        );
    }
}
