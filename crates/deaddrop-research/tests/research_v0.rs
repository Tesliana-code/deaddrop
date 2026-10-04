//! The Research peer on real `Shell`s over an in-process relay, beside
//! Klodik and GitHub. Research's CLI is a fake executable that records its
//! arguments, stdin and each call; nothing reaches the web or a model.

use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use deaddrop_github::Inspector;
use deaddrop_klodik::model::{Model, ModelError, Prompt};
use deaddrop_klodik::{Handled, Peer, State};
use deaddrop_protocol::{DeliveryEventKind, EnvelopeV0, MessageId, MessageKind, NodeId};
use deaddrop_research::{MAX_INBOUND_CHARS, MAX_REPLY_CHARS, ResearchCli};
use deaddrop_shell::{MemoryRelay, Relay, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const DANIL: &str = "danil:local:deaddrop";
const RESEARCH: &str = "research:agent:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const GITHUB: &str = "github:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";
const SHA: &str = "154f992cc3e88f51a0b6bdbf42998e94c3aeedaf";

const FOUND: &str = r#"{"type":"result","subtype":"success","is_error":false,"structured_output":{"findings":"Rust 1.99.0 is the latest stable release (source-supported).","sources":[{"title":"Rust Programming Language","url":"https://www.rust-lang.org/"},{"title":"Announcing Rust 1.99.0","url":"https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/"}]}}"#;

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
    /// Iva and Danil trust all three agents and are trusted back; each agent
    /// allows only Iva.
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-research-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let open = |who: &str| {
            let home = root.join(who);
            init(&home, node(who), RELAY).unwrap();
            Shell::open_with(&home, relay).unwrap()
        };
        let (iva, danil) = (open(IVA), open(DANIL));
        for agent in [RESEARCH, KLODIK, GITHUB].map(open) {
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

    fn peer<A>(&self, who: &str, answer: A) -> Peer<&'r MemoryRelay, A> {
        Peer {
            shell: Shell::open_with(&self.root.join(who), self.relay).unwrap(),
            model: answer,
            state: State::load(&self.root.join(who).join("peer-state.json")).unwrap(),
            allowed: vec![node(IVA)],
            rooms: Vec::new(),
            may_ask: Vec::new(),
        }
    }

    /// A fake CLI: records args and stdin per call, then runs `tail`.
    fn cli(&self, name: &str, tail: &str) -> (ResearchCli, PathBuf) {
        let log = self.root.join(format!("{name}.log"));
        let path = self.root.join(name);
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\necho CALL >> {log}\nfor a in \"$@\"; do printf '[%s]\\n' \"$a\"; done >> {log}\ncat >> {log}\necho >> {log}\n{tail}\n",
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cli = ResearchCli {
            program: path,
            model: None,
            timeout: Duration::from_secs(2),
            workdir: self.root.join("empty"),
            profile: deaddrop_research::Profile::Deep,
        };
        (cli, log)
    }

    fn research(&self, cli: ResearchCli) -> Peer<&'r MemoryRelay, ResearchCli> {
        self.peer(RESEARCH, cli)
    }

    fn iva_to(&self, to: &str, body: &str) -> MessageId {
        self.iva.send(&node(to), body, None, &[]).unwrap()
    }

    fn iva_inbox(&self) -> Vec<EnvelopeV0> {
        self.iva.sync().unwrap();
        self.iva.inbox().unwrap()
    }

    fn acked(&self, id: &MessageId) -> bool {
        self.iva.sync().unwrap();
        self.iva
            .delivery(id)
            .unwrap()
            .iter()
            .any(|e| e.kind() == DeliveryEventKind::RecipientAcknowledged)
    }
}

fn calls(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .matches("CALL\n")
        .count()
}

fn found() -> String {
    format!("printf '%s' '{FOUND}'")
}

#[test]
fn one_question_one_search_one_signed_correlated_reply_with_sources() {
    let relay = MemoryRelay::default();
    let world = World::new("one", &relay);
    let (cli, log) = world.cli("claude", &found());
    let mut research = world.research(cli);
    let asked = world.iva_to(RESEARCH, "research the current Rust stable release");

    assert_eq!(research.step().unwrap().replied.len(), 1);
    assert_eq!(calls(&log), 1);
    let log = std::fs::read_to_string(&log).unwrap();
    assert!(log.contains("[--tools]\n[WebSearch]\n"), "web search only");
    assert!(!log.contains("[Bash]") && !log.contains("WebFetch"));
    assert!(log.contains("Research request from iva:local:deaddrop"));

    let [reply] = world.iva_inbox().try_into().unwrap();
    assert_eq!(
        reply.from(),
        &node(RESEARCH),
        "signed by Research, verified by Iva"
    );
    assert_eq!(reply.kind(), MessageKind::Message);
    assert_eq!(reply.correlation_id().unwrap().as_str(), asked.as_str());
    let body = reply.body();
    assert!(body.starts_with("Rust 1.99.0 is the latest stable release"));
    assert!(body.contains("\n   https://www.rust-lang.org/\n"), "{body}");
    assert!(body.ends_with("\n   https://blog.rust-lang.org/2026/10/01/Rust-1.99.0/"));
    assert!(world.acked(&asked));
}

#[test]
fn failures_send_nothing_but_the_question_is_still_acked() {
    let relay = MemoryRelay::default();
    let world = World::new("fails", &relay);
    for (name, tail) in [
        ("crash", "echo 'not logged in' >&2; exit 1"),
        ("hang", "sleep 10"),
        (
            "unstructured",
            r#"printf '%s' '{"type":"result","subtype":"success","is_error":false,"result":"text"}'"#,
        ),
        (
            "blank",
            r#"printf '%s' '{"type":"result","subtype":"success","is_error":false,"structured_output":{"findings":" ","sources":[]}}'"#,
        ),
    ] {
        let (cli, _) = world.cli(name, tail);
        let mut research = world.research(cli);
        let asked = world.iva_to(RESEARCH, &format!("question for {name}"));
        let step = research.step().unwrap();
        assert!(step.replied.is_empty(), "{name}");
        assert!(world.acked(&asked), "{name}: ACK is receipt only");
        assert!(matches!(
            research.state.get(asked.as_str()),
            Some(Handled::Failed { .. })
        ));
    }
    assert!(world.iva_inbox().is_empty(), "no stand-in answer");
}

#[test]
fn replay_and_restart_search_once() {
    let relay = MemoryRelay::default();
    let world = World::new("restart", &relay);
    let (cli, log) = world.cli("claude", &found());
    world.iva_to(RESEARCH, "find primary sources for X");
    let mut research = world.research(cli.clone());
    research.step().unwrap();
    research.step().unwrap();
    world.research(cli.clone()).step().unwrap();
    std::fs::remove_file(world.root.join(RESEARCH).join("peer-state.json")).unwrap();
    world.research(cli).step().unwrap();
    assert_eq!(calls(&log), 1);
    assert_eq!(world.iva_inbox().len(), 1);
}

#[test]
fn only_iva_may_ask() {
    let relay = MemoryRelay::default();
    let world = World::new("allow", &relay);
    let (cli, log) = world.cli("claude", &found());
    let mut research = world.research(cli);
    let from_danil = world
        .danil
        .send(&node(RESEARCH), "research X", None, &[])
        .unwrap();
    assert_eq!(research.step().unwrap().refused, [from_danil.to_string()]);
    assert_eq!(calls(&log), 0);
}

#[test]
fn unverified_questions_never_reach_the_runtime() {
    let relay = MemoryRelay::default();
    let world = World::new("unverified", &relay);
    let (cli, log) = world.cli("claude", &found());
    let mut research = world.research(cli);
    let forged = EnvelopeV0::new(
        MessageId::generate(),
        node(IVA),
        node(RESEARCH),
        MessageKind::Message,
        "research X",
    );
    relay.post_message(&forged).unwrap();
    research.step().unwrap();
    assert_eq!(calls(&log), 0);
}

#[test]
fn question_and_reply_are_bounded() {
    let relay = MemoryRelay::default();
    let world = World::new("bounds", &relay);
    let huge = "z".repeat(MAX_REPLY_CHARS * 2);
    let (cli, log) = world.cli(
        "claude",
        &format!(
            r#"printf '%s' '{{"type":"result","subtype":"success","is_error":false,"structured_output":{{"findings":"{huge}","sources":[{{"title":"T","url":"https://example.org/a"}}]}}}}'"#
        ),
    );
    let mut research = world.research(cli);
    world.iva_to(RESEARCH, &"q".repeat(MAX_INBOUND_CHARS + 500));
    research.step().unwrap();
    let sent = std::fs::read_to_string(&log).unwrap();
    assert!(sent.contains("[…cut]"), "question cut");
    assert!(sent.matches('q').count() < MAX_INBOUND_CHARS + 100);
    let [reply] = world.iva_inbox().try_into().unwrap();
    assert!(reply.body().chars().count() <= MAX_REPLY_CHARS);
    assert!(
        reply.body().ends_with("https://example.org/a"),
        "source kept"
    );
}

struct Fake(Cell<usize>);

impl Model for Fake {
    fn reply(&self, _: &Prompt) -> Result<String, ModelError> {
        self.0.set(self.0.get() + 1);
        Ok("klodik here".into())
    }
}

fn inspector(world: &World<'_>) -> (Inspector, PathBuf) {
    let log = world.root.join("inspector.log");
    let path = world.root.join("inspector");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\necho CALL >> {}\ncat > /dev/null\nprintf '%s' '{{\"claim\":{{\"commit_modifies_path\":true}},\"evidence\":{{\"repository\":\"o/r\",\"commit\":\"{SHA}\",\"path\":\"x\"}}}}'\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let inspector = Inspector {
        program: path,
        timeout: Duration::from_secs(2),
        default_repo: None,
    };
    (inspector, log)
}

#[test]
fn three_agents_coexist_and_answer_only_their_own_messages() {
    let relay = MemoryRelay::default();
    let world = World::new("three", &relay);
    let (cli, research_log) = world.cli("claude", &found());
    let (gh, github_log) = inspector(&world);
    let mut research = world.research(cli);
    let mut klodik = world.peer(KLODIK, Fake(Cell::new(0)));
    let mut github = world.peer(GITHUB, gh);

    let to_klodik = world.iva_to(KLODIK, "hi klodik");
    let to_github = world.iva_to(GITHUB, &format!("does o/r@{SHA} modify x?"));
    let to_research = world.iva_to(RESEARCH, "research X");
    for _ in 0..2 {
        research.step().unwrap();
        klodik.step().unwrap();
        github.step().unwrap();
    }
    assert_eq!(calls(&research_log), 1);
    assert_eq!(calls(&github_log), 1);
    assert_eq!(klodik.model.0.get(), 1);

    let inbox = world.iva_inbox();
    assert_eq!(inbox.len(), 3);
    let author = |asked: &MessageId| {
        inbox
            .iter()
            .find(|m| m.correlation_id().map(|c| c.as_str()) == Some(asked.as_str()))
            .map(|m| m.from().to_string())
            .unwrap()
    };
    assert_eq!(author(&to_klodik), KLODIK);
    assert_eq!(author(&to_github), GITHUB);
    assert_eq!(author(&to_research), RESEARCH);
    for asked in [&to_klodik, &to_github, &to_research] {
        assert!(world.acked(asked));
    }
}

#[test]
fn a_failing_research_runtime_is_not_covered_for_by_others() {
    let relay = MemoryRelay::default();
    let world = World::new("no-cover", &relay);
    let (cli, _) = world.cli("claude", "exit 1");
    let (gh, github_log) = inspector(&world);
    let mut research = world.research(cli);
    let mut klodik = world.peer(KLODIK, Fake(Cell::new(0)));
    let mut github = world.peer(GITHUB, gh);
    let asked = world.iva_to(RESEARCH, "research X");
    for _ in 0..3 {
        research.step().unwrap();
        klodik.step().unwrap();
        github.step().unwrap();
    }
    assert_eq!(klodik.model.0.get(), 0);
    assert_eq!(calls(&github_log), 0);
    assert!(
        !world
            .iva_inbox()
            .iter()
            .any(|m| m.correlation_id().map(|c| c.as_str()) == Some(asked.as_str()))
    );
}

#[test]
fn agents_never_author_messages_as_the_human() {
    let relay = MemoryRelay::default();
    let world = World::new("human-silent", &relay);
    let (cli, research_log) = world.cli("claude", &found());
    let (gh, github_log) = inspector(&world);
    // Start, sync, restart and sync again: no human input at all.
    for _ in 0..2 {
        let mut research = world.research(cli.clone());
        let mut klodik = world.peer(KLODIK, Fake(Cell::new(0)));
        let mut github = world.peer(GITHUB, gh.clone());
        for _ in 0..3 {
            research.step().unwrap();
            klodik.step().unwrap();
            github.step().unwrap();
        }
        assert_eq!(klodik.model.0.get(), 0);
    }
    assert_eq!(calls(&research_log), 0);
    assert_eq!(calls(&github_log), 0);
    world.iva.sync().unwrap();
    assert!(world.iva.sent().unwrap().is_empty(), "nothing sent as Iva");
    assert!(world.iva.inbox().unwrap().is_empty());
    // Nor anything on the relay claiming to come from Iva.
    assert!(relay.list_for(&node(KLODIK)).unwrap().is_empty());
    assert!(relay.list_for(&node(GITHUB)).unwrap().is_empty());
    assert!(relay.list_for(&node(RESEARCH)).unwrap().is_empty());
}
