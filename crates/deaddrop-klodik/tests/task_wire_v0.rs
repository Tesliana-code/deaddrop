//! /task::wire V0, headless: the TUI's own composer and task path as the
//! orchestrating node, over real `Shell`s, with the three real agent loops
//! (fake runtimes). The planner's proposal is fixed; everything after it —
//! validation, Agent Wire, dispatch, reports, completion — is the real code.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use deaddrop_klodik::model::ModelError;
use deaddrop_klodik::{Answer, Peer, State};
use deaddrop_protocol::{EnvelopeV0, NodeId};
use deaddrop_room::{Kind, RoomConfig, RoomMessage};
use deaddrop_shell::{MemoryRelay, Shell, init};
use deaddrop_task::plan::{Proposed, ProposedStep};
use deaddrop_tui::app::{Action, App, Key, TaskRequest};
use deaddrop_tui::snapshot::load;

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const RESEARCH: &str = "research:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";
const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";
const TASK: &str = "Find the latest official Rust release, inspect whether commit 154f992cc3e88f51a0b6bdbf42998e94c3aeedaf in Tesliana-code/deaddrop modifies crates/deaddrop-tui/src/ui.rs, and summarize both findings.";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

/// A worker runtime: records what it was given, answers with `reply`.
#[derive(Clone)]
struct Runtime {
    calls: Rc<RefCell<Vec<String>>>,
    reply: Result<&'static str, &'static str>,
}

impl Runtime {
    fn ok(reply: &'static str) -> Self {
        Self {
            calls: Rc::default(),
            reply: Ok(reply),
        }
    }
    fn failing() -> Self {
        Self {
            calls: Rc::default(),
            reply: Err("search provider down"),
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Answer for Runtime {
    fn answer(&self, m: &EnvelopeV0) -> Result<String, ModelError> {
        self.calls.borrow_mut().push(m.body().to_owned());
        self.reply
            .map(str::to_owned)
            .map_err(|e| ModelError::Malformed(e.into()))
    }
}

const FOUND: &str = "Rust 1.99.0 is the latest stable release (October 1, 2026).\n\nSources:\n1. Announcing Rust 1.99.0\n   https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/";
const YES: &str = "yes, it modifies crates/deaddrop-tui/src/ui.rs\nTesliana-code/deaddrop@154f992cc3e88f51a0b6bdbf42998e94c3aeedaf\nsource: github.inspect · commit_modifies_path (github-inspector, read-only)";
const SUMMARY: &str = "Rust 1.99.0 is out, and commit 154f992 does modify ui.rs.";

fn golden() -> Proposed {
    let step = |id: &str, capability: &str, objective: String, deps: &[&str]| ProposedStep {
        id: id.into(),
        capability: capability.into(),
        objective,
        depends_on: deps.iter().map(|d| (*d).to_owned()).collect(),
    };
    Proposed {
        steps: vec![
            step(
                "research_release",
                "web.search",
                "What is the latest official stable Rust release?".into(),
                &[],
            ),
            step(
                "inspect_commit",
                "github.inspect",
                format!("does Tesliana-code/deaddrop@{SHA} modify crates/deaddrop-tui/src/ui.rs?"),
                &[],
            ),
            step(
                "synthesize",
                "synthesize",
                "Summarize both findings.".into(),
                &["research_release", "inspect_commit"],
            ),
        ],
    }
}

struct World<'r> {
    root: PathBuf,
    relay: &'r MemoryRelay,
    iva: Shell<&'r MemoryRelay>,
    room: RoomConfig,
}

impl<'r> World<'r> {
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-task-wire-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        let members = [IVA, KLODIK, GITHUB, RESEARCH];
        let shells: Vec<_> = members
            .map(|who| {
                let home = root.join(who);
                init(&home, node(who), RELAY).unwrap();
                Shell::open_with(&home, relay).unwrap()
            })
            .into();
        for a in &shells {
            for b in &shells {
                let card = b.identity();
                if card.node != a.identity().node {
                    a.add_peer(&card.node, &card.key).unwrap();
                }
            }
        }
        Self {
            iva: shells.into_iter().next().unwrap(),
            root,
            relay,
            room: RoomConfig {
                name: "d34ddr0p".into(),
                members: members.map(str::to_owned).to_vec(),
            },
        }
    }

    fn peer(&self, who: &str, rt: Runtime) -> Peer<&'r MemoryRelay, Runtime> {
        let home = self.root.join(who);
        Peer {
            shell: Shell::open_with(&home, self.relay).unwrap(),
            model: rt,
            state: State::load(&home.join("peer/state.json")).unwrap(),
            allowed: vec![node(IVA)],
            rooms: vec![self.room.clone()],
            may_ask: Vec::new(),
        }
    }

    fn app(&self, may_ask: &[&str]) -> App {
        let mut app = App::new();
        app.set_rooms(vec![self.room.clone()]);
        app.task_policy = may_ask.iter().map(|p| (*p).to_owned()).collect();
        self.sync(&mut app);
        app.key(Key::ClickRoom(0));
        app
    }

    /// A sync, then what main does after one: advance tasks, send.
    fn sync(&self, app: &mut App) {
        app.begin_refresh();
        app.finish(Ok(load(&self.iva).unwrap()));
        app.advance_tasks();
        self.deliver(app);
    }

    /// Deliver every task message as main does. Returns how many.
    fn deliver(&self, app: &mut App) -> usize {
        let sends = app.take_task_sends();
        for out in &sends {
            let results: Vec<_> = out
                .to
                .iter()
                .map(|m| {
                    let sent = self.iva.send(&node(m), &out.body, None, &[]);
                    (
                        m.clone(),
                        sent.map(|id| id.to_string()).map_err(|e| e.to_string()),
                    )
                })
                .collect();
            app.finish_task_send(out, &results);
        }
        sends.len()
    }

    /// Type the task and press Enter: the planner is asked, nothing else.
    fn enter(&self, app: &mut App, command: &str) -> TaskRequest {
        for c in TASK.chars() {
            app.key(Key::Char(c));
        }
        app.key(Key::Newline);
        for c in command.chars() {
            app.key(Key::Char(c));
        }
        let Action::PlanTask(request) = app.key(Key::Enter) else {
            panic!("a task goes to the planner: {}", app.status)
        };
        assert_eq!(request.payload, TASK, "the parser's payload, unchanged");
        assert!(
            app.take_task_sends().is_empty(),
            "nothing sent before a plan"
        );
        request
    }

    /// Iva's room messages: (kind, text).
    fn said(&self) -> Vec<(Kind, String)> {
        let mut seen = Vec::new();
        let mut out = Vec::new();
        for s in self.iva.sent().unwrap() {
            if let Some(Ok(m)) = RoomMessage::decode(s.body())
                && !seen.contains(&m.id)
            {
                seen.push(m.id.clone());
                out.push((m.kind, m.text));
            }
        }
        out
    }
}

#[test]
fn the_golden_task_runs_concurrently_synthesizes_and_completes_once() {
    let relay = MemoryRelay::default();
    let w = World::new("golden", &relay);
    let (research_rt, github_rt, klodik_rt) =
        (Runtime::ok(FOUND), Runtime::ok(YES), Runtime::ok(SUMMARY));
    let mut research = w.peer(RESEARCH, research_rt.clone());
    let mut github = w.peer(GITHUB, github_rt.clone());
    let mut klodik = w.peer(KLODIK, klodik_rt.clone());
    let mut app = w.app(&[RESEARCH, GITHUB, KLODIK]);

    let request = w.enter(&mut app, "/task::wire");
    app.planned(&request.id, Ok(golden()));
    assert_eq!(
        w.deliver(&mut app),
        4,
        "your task, the wire's acceptance, then both independent steps"
    );
    let said = w.said();
    let accepted = format!("task:: accepted\nid:: {}\nsteps:: 3", request.id);
    assert!(
        said.iter()
            .any(|(k, t)| *k == Kind::Message && *t == accepted)
    );
    let yours = format!("{TASK}\n\n/task::wire");
    assert!(
        said.iter().any(|(_, t)| *t == yours),
        "your words, as written"
    );
    assert_eq!(said.iter().filter(|(k, _)| *k == Kind::Request).count(), 2);
    assert!(
        said.iter().all(|(_, t)| !t.contains('@')),
        "no @mentions anywhere"
    );

    // One round: research and github both run; klodik has nothing yet.
    w.sync(&mut app);
    let mut pending: Vec<String> = app.pending().into_iter().map(|p| p.member).collect();
    pending.sort();
    assert_eq!(pending, [GITHUB, RESEARCH], "both out at once");
    research.step().unwrap();
    github.step().unwrap();
    klodik.step().unwrap();
    assert_eq!(research_rt.calls().len(), 1);
    assert_eq!(github_rt.calls().len(), 1);
    assert!(
        github_rt.calls()[0].contains("request:: github.inspect\nrepo:: Tesliana-code/deaddrop")
    );
    assert!(
        klodik_rt.calls().is_empty(),
        "synthesis waits for both reports"
    );

    // Both reports in: synthesis is assigned and sent; the next sync (main
    // starts one after every task send) shows it out.
    w.sync(&mut app);
    w.sync(&mut app);
    let pending: Vec<String> = app.pending().into_iter().map(|p| p.member).collect();
    assert_eq!(pending, [KLODIK], "synthesis is now out");
    klodik.step().unwrap();
    let given = &klodik_rt.calls()[0];
    assert!(given.contains("REPORT research_release (web.search · research)\nRust 1.99.0"));
    assert!(given.contains("REPORT inspect_commit (github.inspect · github)\nyes, it modifies"));
    assert!(
        !given.contains("task:: accepted"),
        "only the reports it depends on"
    );

    w.sync(&mut app);
    for _ in 0..3 {
        research.step().unwrap();
        github.step().unwrap();
        klodik.step().unwrap();
        w.sync(&mut app);
    }
    assert_eq!(
        (
            research_rt.calls().len(),
            github_rt.calls().len(),
            klodik_rt.calls().len()
        ),
        (1, 1, 1),
        "each worker ran exactly once"
    );
    let completes = w
        .said()
        .into_iter()
        .filter(|(_, t)| t.starts_with("task:: complete"))
        .count();
    assert_eq!(completes, 1, "complete exactly once");
    assert!(app.pending().is_empty());

    // The wire trace, in order of what really happened.
    let traces = app.task_traces("d34ddr0p");
    let [(id, events)] = traces.as_slice() else {
        panic!("one task")
    };
    assert_eq!(*id, request.id);
    let lines: Vec<String> = events
        .iter()
        .map(|e| {
            let to =
                e.to.as_deref()
                    .map(|t| format!(" → {t}"))
                    .unwrap_or_default();
            format!("{}{to} {}:: {}", e.from, e.fields[0].0, e.fields[0].1)
        })
        .collect();
    assert_eq!(
        lines[..3],
        [
            "orchestrator plan:: accepted · 3 steps",
            "orchestrator → research assign:: web.search",
            "orchestrator → github assign:: github.inspect",
        ]
    );
    // The two independent reports, in whichever order they arrived, each
    // followed by its evaluation.
    let mut branches = [lines[3..5].join(" | "), lines[5..7].join(" | ")];
    branches.sort();
    assert_eq!(
        branches,
        [
            "github → orchestrator report:: inspect_commit | orchestrator evaluate:: inspect_commit · gates met · COMPLETE",
            "research → orchestrator report:: research_release | orchestrator evaluate:: research_release · gates met · COMPLETE",
        ]
    );
    assert_eq!(
        lines[7..],
        [
            "orchestrator → klodik assign:: synthesize",
            "klodik → orchestrator report:: synthesize",
            "orchestrator evaluate:: synthesize · gates met · COMPLETE",
            "orchestrator evaluate:: pass",
            &format!("orchestrator complete:: {}", request.id),
        ]
    );
}

#[test]
fn a_dry_run_plans_and_runs_nothing() {
    let relay = MemoryRelay::default();
    let w = World::new("dry-run", &relay);
    let rts = [Runtime::ok(FOUND), Runtime::ok(YES), Runtime::ok(SUMMARY)];
    let mut peers: Vec<_> = [RESEARCH, GITHUB, KLODIK]
        .iter()
        .zip(&rts)
        .map(|(who, rt)| w.peer(who, rt.clone()))
        .collect();
    let mut app = w.app(&[RESEARCH, GITHUB, KLODIK]);
    let request = w.enter(&mut app, "/task::wire --dry-run");
    app.planned(&request.id, Ok(golden()));
    w.sync(&mut app);
    for p in &mut peers {
        p.step().unwrap();
    }
    assert!(
        rts.iter().all(|rt| rt.calls().is_empty()),
        "zero worker calls"
    );
    assert!(w.iva.sent().unwrap().is_empty(), "nothing sent at all");
    let note: Vec<String> = app
        .task_notes("d34ddr0p")
        .flat_map(|n| n.lines.clone())
        .collect();
    assert_eq!(
        note[0],
        format!("task:: dry-run · {} · nothing was run", request.id)
    );
    assert!(note.contains(&"  synthesize <- research_release, inspect_commit".to_owned()));
    assert!(note.contains(&"workers:: research, github, klodik".to_owned()));
}

#[test]
fn policy_decides_not_capability() {
    let relay = MemoryRelay::default();
    let w = World::new("policy", &relay);
    let klodik_rt = Runtime::ok(SUMMARY);
    let mut klodik = w.peer(KLODIK, klodik_rt.clone());
    // Klodik can synthesize, but this node's policy does not let tasks ask it.
    let mut app = w.app(&[RESEARCH, GITHUB]);
    let request = w.enter(&mut app, "/task::wire");
    app.planned(&request.id, Ok(golden()));
    w.sync(&mut app);
    klodik.step().unwrap();
    assert!(
        w.iva.sent().unwrap().is_empty(),
        "a rejected plan asks no one"
    );
    assert!(klodik_rt.calls().is_empty());
    let note: Vec<String> = app
        .task_notes("d34ddr0p")
        .flat_map(|n| n.lines.clone())
        .collect();
    assert_eq!(
        note,
        [
            format!("task:: rejected · {}", request.id),
            "reason:: step synthesize: synthesize: policy does not allow asking klodik".into(),
        ]
    );
}

#[test]
fn a_failed_branch_keeps_the_other_report_and_blocks_synthesis() {
    let relay = MemoryRelay::default();
    let w = World::new("failure", &relay);
    let (research_rt, github_rt, klodik_rt) =
        (Runtime::failing(), Runtime::ok(YES), Runtime::ok(SUMMARY));
    let mut research = w.peer(RESEARCH, research_rt.clone());
    let mut github = w.peer(GITHUB, github_rt.clone());
    let mut klodik = w.peer(KLODIK, klodik_rt.clone());
    let mut app = w.app(&[RESEARCH, GITHUB, KLODIK]);
    let request = w.enter(&mut app, "/task::wire");
    app.planned(&request.id, Ok(golden()));
    w.deliver(&mut app);
    // The agent loop retries a failing runtime before it reports failure.
    for _ in 0..4 {
        research.step().unwrap();
        github.step().unwrap();
        klodik.step().unwrap();
        w.sync(&mut app);
    }
    assert!(klodik_rt.calls().is_empty(), "synthesis never assigned");
    let said = w.said();
    let failed: Vec<&String> = said
        .iter()
        .map(|(_, t)| t)
        .filter(|t| t.starts_with("task:: failed"))
        .collect();
    assert_eq!(failed.len(), 1);
    assert!(failed[0].contains("research_release failed (research failed); synthesize blocked"));
    // GitHub's result stands, in the room.
    let thread: Vec<String> = app
        .current_thread()
        .iter()
        .map(|r| r.body().to_owned())
        .collect();
    assert!(thread.iter().any(|b| b.starts_with("yes, it modifies")));
    assert!(!said.iter().any(|(_, t)| t.starts_with("task:: complete")));
}
