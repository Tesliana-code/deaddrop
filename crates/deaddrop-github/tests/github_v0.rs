//! The GitHub peer and Klodik side by side on real `Shell`s over an
//! in-process relay. The inspector is a fake executable that records each
//! call; Klodik's model is a fake. Nothing reaches GitHub or a model.

use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use deaddrop_github::Inspector;
use deaddrop_klodik::model::{Model, ModelError, Prompt};
use deaddrop_klodik::{Handled, Peer, State};
use deaddrop_protocol::{DeliveryEventKind, EnvelopeV0, MessageId, MessageKind, NodeId};
use deaddrop_shell::{MemoryRelay, Relay, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const DANIL: &str = "danil:local:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";
const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

struct World<'r> {
    root: PathBuf,
    relay: &'r MemoryRelay,
    iva: Shell<&'r MemoryRelay>,
    danil: Shell<&'r MemoryRelay>,
}

impl<'r> World<'r> {
    /// Iva and Danil trust both agents and both agents trust them; each
    /// agent allows only Iva.
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-github-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let open = |who: &str| {
            let home = root.join(who);
            init(&home, node(who), RELAY).unwrap();
            Shell::open_with(&home, relay).unwrap()
        };
        let (iva, danil, github, klodik) = (open(IVA), open(DANIL), open(GITHUB), open(KLODIK));
        for agent in [&github, &klodik] {
            let a = agent.identity();
            for human in [&iva, &danil] {
                let h = human.identity();
                human.add_peer(&a.node, &a.key).unwrap();
                agent.add_peer(&h.node, &h.key).unwrap();
            }
        }
        Self {
            root,
            relay,
            iva,
            danil,
        }
    }

    fn shell(&self, who: &str) -> Shell<&'r MemoryRelay> {
        Shell::open_with(&self.root.join(who), self.relay).unwrap()
    }

    /// A GitHub peer process over its home: building another is a restart.
    fn github(&self, inspector: Inspector) -> Peer<&'r MemoryRelay, Inspector> {
        Peer {
            shell: self.shell(GITHUB),
            model: inspector,
            state: State::load(&self.root.join(GITHUB).join("github-peer/state.json")).unwrap(),
            allowed: vec![node(IVA)],
            rooms: Vec::new(),
            may_ask: Vec::new(),
        }
    }

    fn klodik<M: Model>(&self, model: M) -> Peer<&'r MemoryRelay, M> {
        Peer {
            shell: self.shell(KLODIK),
            model,
            state: State::load(&self.root.join(KLODIK).join("klodik/state.json")).unwrap(),
            allowed: vec![node(IVA)],
            rooms: Vec::new(),
            may_ask: Vec::new(),
        }
    }

    fn iva_to(&self, to: &str, body: &str) -> MessageId {
        self.iva.send(&node(to), body, None, &[]).unwrap()
    }

    fn iva_inbox(&self) -> Vec<EnvelopeV0> {
        self.iva.sync().unwrap();
        self.iva.inbox().unwrap()
    }

    /// A fake inspector: logs each call's stdin, then runs `tail`.
    fn inspector(&self, name: &str, tail: &str) -> (Inspector, PathBuf) {
        let calls = self.root.join(format!("{name}.calls"));
        let path = self.root.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\ncat >> {}\necho >> {}\n{tail}\n",
                calls.display(),
                calls.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let inspector = Inspector {
            program: path,
            timeout: Duration::from_secs(2),
            default_repo: None,
        };
        (inspector, calls)
    }
}

fn calls(log: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

const YES: &str = r#"printf '%s' '{"claim":{"commit_modifies_path":true},"evidence":{"repository":"Tesliana-code/deaddrop","commit":"154f992cc3e88f51a0b6bdbf42998e94c3aeedaf","path":"crates/deaddrop-tui/src/ui.rs"}}'"#;

fn question() -> String {
    format!("does Tesliana-code/deaddrop@{SHA} modify crates/deaddrop-tui/src/ui.rs?")
}

struct Fake {
    calls: Cell<usize>,
    answer: Result<&'static str, ModelError>,
}

impl Model for Fake {
    fn reply(&self, _: &Prompt) -> Result<String, ModelError> {
        self.calls.set(self.calls.get() + 1);
        self.answer.clone().map(str::to_owned)
    }
}

fn fake(answer: Result<&'static str, ModelError>) -> Fake {
    Fake {
        calls: Cell::new(0),
        answer,
    }
}

#[test]
fn a_supported_question_runs_the_worker_once_and_replies_with_its_evidence() {
    let relay = MemoryRelay::default();
    let world = World::new("supported", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    let asked = world.iva_to(GITHUB, &question());

    assert_eq!(github.step().unwrap().replied.len(), 1);
    let calls = calls(&log);
    assert_eq!(
        calls,
        [serde_json::json!({
            "operation": "commit_modifies_path",
            "repository": "Tesliana-code/deaddrop",
            "commit": SHA,
            "path": "crates/deaddrop-tui/src/ui.rs",
        })],
        "one call, in the worker's own schema"
    );

    let [reply] = world.iva_inbox().try_into().unwrap();
    assert_eq!(reply.from(), &node(GITHUB));
    assert_eq!(reply.kind(), MessageKind::Message);
    assert_eq!(reply.correlation_id().unwrap().as_str(), asked.as_str());
    assert!(
        reply
            .body()
            .starts_with("yes, it modifies crates/deaddrop-tui/src/ui.rs")
    );
    assert!(
        reply
            .body()
            .contains(&format!("Tesliana-code/deaddrop@{SHA}"))
    );
    let acked = world
        .iva
        .delivery(&asked)
        .unwrap()
        .iter()
        .any(|e| e.kind() == DeliveryEventKind::RecipientAcknowledged);
    assert!(acked);
}

#[test]
fn unsupported_requests_get_a_truthful_refusal_and_no_worker_call() {
    let relay = MemoryRelay::default();
    let world = World::new("unsupported", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    world.iva_to(GITHUB, "please merge my PR");
    world.iva_to(GITHUB, "what's the weather?");

    assert_eq!(github.step().unwrap().replied.len(), 2);
    assert!(calls(&log).is_empty(), "the worker never ran");
    for reply in world.iva_inbox() {
        assert!(
            reply.body().starts_with("Not supported"),
            "{}",
            reply.body()
        );
        assert!(reply.body().contains("read-only"));
    }
}

#[test]
fn replay_and_restart_do_not_run_the_worker_again() {
    let relay = MemoryRelay::default();
    let world = World::new("restart", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    world.iva_to(GITHUB, &question());
    let mut github = world.github(inspector.clone());
    github.step().unwrap();
    github.step().unwrap();
    world.github(inspector.clone()).step().unwrap();
    std::fs::remove_file(world.root.join(GITHUB).join("github-peer/state.json")).unwrap();
    world.github(inspector).step().unwrap();
    assert_eq!(calls(&log).len(), 1);
    assert_eq!(world.iva_inbox().len(), 1);
}

#[test]
fn worker_breakage_sends_nothing_but_worker_verdicts_are_reported() {
    let relay = MemoryRelay::default();
    let world = World::new("worker-fails", &relay);
    let mut asked = Vec::new();
    for (name, tail) in [
        ("crash", "echo 'segfault' >&2; exit 101"),
        ("garbage", "echo pong"),
        ("hang", "sleep 10"),
    ] {
        let (inspector, _) = world.inspector(name, tail);
        let mut github = world.github(inspector);
        asked.push(world.iva_to(GITHUB, &question()));
        let step = github.step().unwrap();
        assert!(step.replied.is_empty(), "{name}");
        assert!(matches!(
            github.state.get(asked.last().unwrap().as_str()),
            Some(Handled::Failed { .. })
        ));
    }
    assert!(world.iva_inbox().is_empty(), "no stand-in reply");

    let (unresolved, _) = world.inspector(
        "unresolved",
        r#"printf '%s' '{"status":"unresolved","error":{"kind":"evidence_unavailable","message":"HTTP 404: No commit found"}}' >&2; exit 3"#,
    );
    let mut github = world.github(unresolved);
    let step = github.step().unwrap();
    // The first message already failed three times and was given up on;
    // the other two are retried and get the worker's verdict.
    assert_eq!(step.replied.len(), 2);
    assert!(matches!(
        github.state.get(asked[0].as_str()),
        Some(Handled::Failed { attempts: 3, .. })
    ));
    for reply in world.iva_inbox() {
        assert!(
            reply
                .body()
                .starts_with("github-inspector unresolved: HTTP 404")
        );
        assert!(reply.body().contains("No answer"));
    }
}

#[test]
fn trusted_but_not_allowed_peers_get_no_worker_execution() {
    let relay = MemoryRelay::default();
    let world = World::new("not-allowed", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    let from_danil = world
        .danil
        .send(&node(GITHUB), &question(), None, &[])
        .unwrap();
    assert_eq!(github.step().unwrap().refused, [from_danil.to_string()]);
    assert!(calls(&log).is_empty());
}

#[test]
fn unverified_messages_never_reach_the_worker() {
    let relay = MemoryRelay::default();
    let world = World::new("unverified", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    let forged = EnvelopeV0::new(
        MessageId::generate(),
        node(IVA),
        node(GITHUB),
        MessageKind::Message,
        question(),
    );
    relay.post_message(&forged).unwrap();
    github.step().unwrap();
    assert!(calls(&log).is_empty());
    assert!(github.shell.inbox().unwrap().is_empty());
}

#[test]
fn agents_coexist_without_cross_delivery() {
    let relay = MemoryRelay::default();
    let world = World::new("coexist", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    let mut klodik = world.klodik(fake(Ok("hi iva")));

    let to_klodik = world.iva_to(KLODIK, &question());
    github.step().unwrap();
    assert!(
        calls(&log).is_empty(),
        "a message for Klodik is not GitHub's"
    );
    klodik.step().unwrap();
    assert_eq!(klodik.model.calls.get(), 1);

    let to_github = world.iva_to(GITHUB, &question());
    klodik.step().unwrap();
    assert_eq!(
        klodik.model.calls.get(),
        1,
        "a message for GitHub is not Klodik's"
    );
    github.step().unwrap();
    assert_eq!(calls(&log).len(), 1);

    let inbox = world.iva_inbox();
    let from = |id: &MessageId| {
        inbox
            .iter()
            .find(|m| m.correlation_id().map(|c| c.as_str()) == Some(id.as_str()))
            .map(|m| m.from().to_string())
    };
    assert_eq!(from(&to_klodik).as_deref(), Some(KLODIK));
    assert_eq!(from(&to_github).as_deref(), Some(GITHUB));
    assert_eq!(inbox.len(), 2);
}

#[test]
fn one_agent_failing_does_not_make_another_answer_for_it() {
    let relay = MemoryRelay::default();
    let world = World::new("no-impersonation", &relay);
    let (inspector, log) = world.inspector("inspector", YES);
    let mut github = world.github(inspector);
    let mut klodik = world.klodik(fake(Err(ModelError::Empty)));
    let to_klodik = world.iva_to(KLODIK, "are you there?");
    for _ in 0..3 {
        klodik.step().unwrap();
        github.step().unwrap();
    }
    assert!(calls(&log).is_empty());
    let inbox = world.iva_inbox();
    assert!(
        !inbox
            .iter()
            .any(|m| m.correlation_id().map(|c| c.as_str()) == Some(to_klodik.as_str())),
        "nobody answered for Klodik"
    );
}
