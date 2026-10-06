//! Klodik over real `Shell`s on an in-process relay, with a fake model.
//! Nothing here reaches the network or a real model.

use std::cell::{Cell, RefCell};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use deaddrop_klodik::model::{ClaudeCli, Model, ModelError, PERSONA, Prompt};
use deaddrop_klodik::{Handled, Klodik, MAX_ATTEMPTS, State};
use deaddrop_protocol::{
    ArtifactRef, DeliveryEventKind, EnvelopeV0, MessageId, MessageKind, NodeId, SignatureV0,
};
use deaddrop_shell::{MemoryRelay, Relay, RelayError, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const DANIL: &str = "danil:local:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

fn dir(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-klodik-tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A relay that can be switched off, to model an outage.
#[derive(Clone)]
struct Switch<'r> {
    relay: &'r MemoryRelay,
    down: Rc<Cell<bool>>,
}

impl Switch<'_> {
    fn check(&self) -> Result<(), RelayError> {
        if self.down.get() {
            Err(RelayError::Failed("down".into()))
        } else {
            Ok(())
        }
    }
}

impl Relay for Switch<'_> {
    fn post_message(&self, e: &EnvelopeV0) -> Result<(), RelayError> {
        self.check()?;
        self.relay.post_message(e)
    }
    fn post_signature(&self, s: &SignatureV0) -> Result<(), RelayError> {
        self.check()?;
        self.relay.post_signature(s)
    }
    fn list_for(&self, n: &NodeId) -> Result<Vec<MessageId>, RelayError> {
        self.check()?;
        self.relay.list_for(n)
    }
    fn get_message(&self, id: &MessageId) -> Result<Option<EnvelopeV0>, RelayError> {
        self.check()?;
        self.relay.get_message(id)
    }
    fn signatures(&self, id: &MessageId) -> Result<Vec<SignatureV0>, RelayError> {
        self.check()?;
        self.relay.signatures(id)
    }
    fn put_artifact(&self, b: &[u8]) -> Result<ArtifactRef, RelayError> {
        self.check()?;
        self.relay.put_artifact(b)
    }
    fn get_artifact(&self, a: &ArtifactRef) -> Result<Option<Vec<u8>>, RelayError> {
        self.check()?;
        self.relay.get_artifact(a)
    }
}

/// A model that records every prompt and answers from a script.
#[derive(Clone, Default)]
struct Fake {
    prompts: Rc<RefCell<Vec<Prompt>>>,
    answers: Rc<RefCell<Vec<Result<String, ModelError>>>>,
}

impl Fake {
    fn answering(answers: Vec<Result<String, ModelError>>) -> Self {
        let fake = Self::default();
        *fake.answers.borrow_mut() = answers;
        fake
    }
    fn calls(&self) -> usize {
        self.prompts.borrow().len()
    }
}

impl Model for Fake {
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError> {
        self.prompts.borrow_mut().push(prompt.clone());
        let mut answers = self.answers.borrow_mut();
        if answers.is_empty() {
            Ok("pong".into())
        } else {
            answers.remove(0)
        }
    }
}

struct World<'r> {
    root: PathBuf,
    relay: &'r MemoryRelay,
    down: Rc<Cell<bool>>,
    iva: Shell<&'r MemoryRelay>,
    danil: Shell<&'r MemoryRelay>,
}

impl<'r> World<'r> {
    /// Iva and Danil both trust Klodik and Klodik trusts both; only Iva is
    /// allowed to talk to it.
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = dir(test);
        let open = |who: &str| {
            let home = root.join(who);
            init(&home, node(who), RELAY).unwrap();
            Shell::open_with(&home, relay).unwrap()
        };
        let (iva, klodik, danil) = (open(IVA), open(KLODIK), open(DANIL));
        let k = klodik.identity();
        for peer in [&iva, &danil] {
            let card = peer.identity();
            peer.add_peer(&k.node, &k.key).unwrap();
            klodik.add_peer(&card.node, &card.key).unwrap();
        }
        Self {
            root,
            relay,
            down: Rc::new(Cell::new(false)),
            iva,
            danil,
        }
    }

    fn state_path(&self) -> PathBuf {
        self.root.join(KLODIK).join("klodik").join("state.json")
    }

    /// A Klodik process over the shared home: building a second one is a
    /// restart.
    fn klodik<M: Model>(&self, model: M) -> Klodik<Switch<'r>, M> {
        let relay = Switch {
            relay: self.relay,
            down: self.down.clone(),
        };
        Klodik {
            shell: Shell::open_with(&self.root.join(KLODIK), relay).unwrap(),
            model,
            state: State::load(&self.state_path()).unwrap(),
            allowed: vec![node(IVA)],
            rooms: Vec::new(),
            may_ask: Vec::new(),
        }
    }

    fn iva_says(&self, body: &str) -> MessageId {
        self.iva.send(&node(KLODIK), body, None, &[]).unwrap()
    }

    /// What Iva has from Klodik after syncing.
    fn iva_inbox(&self) -> Vec<EnvelopeV0> {
        self.iva.sync().unwrap();
        self.iva.inbox().unwrap()
    }
}

#[test]
fn one_verified_message_gets_one_model_call_and_one_signed_reply() {
    let relay = MemoryRelay::default();
    let world = World::new("one-reply", &relay);
    let fake = Fake::default();
    let mut klodik = world.klodik(fake.clone());
    let ping = world.iva_says("ping klodik");

    let step = klodik.step().unwrap();
    assert_eq!(step.replied.len(), 1, "{step:?}");
    assert_eq!(fake.calls(), 1);
    let prompt = &fake.prompts.borrow()[0];
    assert_eq!(prompt.system, PERSONA, "persona travels with every call");
    assert!(prompt.user.contains("ping klodik") && prompt.user.contains(IVA));

    // Iva verifies the reply through the normal sync path.
    let [reply] = world.iva_inbox().try_into().unwrap();
    assert_eq!(reply.from(), &node(KLODIK));
    assert_eq!(reply.body(), "pong");
    assert_eq!(reply.kind(), MessageKind::Message, "an ordinary message");
    assert_eq!(
        reply.correlation_id().map(|c| c.as_str()),
        Some(ping.as_str()),
        "correlated to what it answers"
    );
    assert_eq!(
        klodik.state.get(ping.as_str()),
        Some(&Handled::Replied {
            reply: reply.id().to_string()
        })
    );
}

#[test]
fn messages_that_fail_verification_never_reach_the_model() {
    let relay = MemoryRelay::default();
    let world = World::new("unverified", &relay);
    let fake = Fake::default();
    let mut klodik = world.klodik(fake.clone());

    // Unsigned, claiming to be Iva.
    let forged = EnvelopeV0::new(
        MessageId::generate(),
        node(IVA),
        node(KLODIK),
        MessageKind::Message,
        "forged",
    );
    relay.post_message(&forged).unwrap();
    // Signed by a stranger Klodik does not trust.
    let home = world.root.join("stranger");
    init(&home, node("stranger:local:deaddrop"), RELAY).unwrap();
    let stranger = Shell::open_with(&home, &relay).unwrap();
    let k = klodik.shell.identity();
    stranger.add_peer(&k.node, &k.key).unwrap();
    stranger.send(&node(KLODIK), "hello?", None, &[]).unwrap();

    let step = klodik.step().unwrap();
    assert_eq!(fake.calls(), 0);
    assert!(step.replied.is_empty());
    assert!(klodik.shell.inbox().unwrap().is_empty(), "nothing kept");
}

#[test]
fn a_replayed_message_is_answered_once() {
    let relay = MemoryRelay::default();
    let world = World::new("replay", &relay);
    let fake = Fake::default();
    let mut klodik = world.klodik(fake.clone());
    world.iva_says("ping");
    for _ in 0..3 {
        klodik.step().unwrap();
    }
    assert_eq!(fake.calls(), 1);
    assert_eq!(world.iva_inbox().len(), 1);
}

#[test]
fn a_restart_does_not_answer_again() {
    let relay = MemoryRelay::default();
    let world = World::new("restart", &relay);
    world.iva_says("ping");
    world.klodik(Fake::default()).step().unwrap();

    let again = Fake::default();
    world.klodik(again.clone()).step().unwrap();
    assert_eq!(again.calls(), 0, "state file remembers");

    // Even with the state file lost, the sent reply is found.
    std::fs::remove_file(world.state_path()).unwrap();
    let lost = Fake::default();
    let mut klodik = world.klodik(lost.clone());
    klodik.step().unwrap();
    assert_eq!(lost.calls(), 0, "Shell::sent has the correlated reply");
    assert!(matches!(
        klodik
            .state
            .get(world.iva_inbox()[0].correlation_id().unwrap().as_str()),
        Some(Handled::Replied { .. })
    ));
    assert_eq!(world.iva_inbox().len(), 1);
}

#[test]
fn model_failure_sends_nothing_and_gives_up_after_the_limit() {
    let relay = MemoryRelay::default();
    let world = World::new("model-fails", &relay);
    let fake = Fake::answering(vec![Err(ModelError::TimedOut(Duration::from_secs(1))); 5]);
    let mut klodik = world.klodik(fake.clone());
    let ping = world.iva_says("ping");

    let step = klodik.step().unwrap();
    assert!(step.replied.is_empty());
    assert!(step.failed[0].1.contains("timed out"));
    for _ in 0..5 {
        klodik.step().unwrap();
    }
    assert_eq!(fake.calls(), MAX_ATTEMPTS as usize, "bounded retries");
    assert!(world.iva_inbox().is_empty(), "no stand-in reply");
    assert!(matches!(
        klodik.state.get(ping.as_str()),
        Some(Handled::Failed { attempts, .. }) if *attempts == MAX_ATTEMPTS
    ));
    // The message itself is still in Klodik's verified inbox.
    assert_eq!(klodik.shell.inbox().unwrap().len(), 1);
}

#[test]
fn an_empty_model_reply_is_not_sent() {
    let relay = MemoryRelay::default();
    let world = World::new("empty", &relay);
    let mut klodik = world.klodik(Fake::answering(vec![Err(ModelError::Empty)]));
    world.iva_says("ping");
    let step = klodik.step().unwrap();
    assert!(step.failed[0].1.contains("empty"));
    assert!(world.iva_inbox().is_empty());
}

#[test]
fn ack_means_received_not_answered() {
    let relay = MemoryRelay::default();
    let world = World::new("ack", &relay);
    let mut klodik = world.klodik(Fake::answering(vec![Err(ModelError::Empty)]));
    let ping = world.iva_says("ping");
    klodik.step().unwrap();

    world.iva.sync().unwrap();
    let acked = world
        .iva
        .delivery(&ping)
        .unwrap()
        .iter()
        .any(|e| e.kind() == DeliveryEventKind::RecipientAcknowledged);
    assert!(acked, "Klodik received it");
    assert!(world.iva.inbox().unwrap().is_empty(), "and did not answer");
}

#[test]
fn only_allowed_peers_reach_the_model() {
    let relay = MemoryRelay::default();
    let world = World::new("allow", &relay);
    let fake = Fake::default();
    let mut klodik = world.klodik(fake.clone());
    let from_danil = world.danil.send(&node(KLODIK), "hi", None, &[]).unwrap();

    let step = klodik.step().unwrap();
    assert_eq!(step.refused, [from_danil.to_string()]);
    assert_eq!(fake.calls(), 0);
    world.danil.sync().unwrap();
    assert!(world.danil.inbox().unwrap().is_empty());
    klodik.step().unwrap();
    assert!(klodik.step().unwrap().refused.is_empty(), "refused once");
}

#[test]
fn a_failed_send_is_retried_without_asking_the_model_again() {
    let relay = MemoryRelay::default();
    let world = World::new("send-fails", &relay);
    let fake = Fake::default();
    let klodik = world.klodik(fake.clone());
    let ping = world.iva_says("ping");
    // Pull and ACK while the relay is up, then lose it before the send.
    klodik.shell.sync().unwrap();
    klodik.shell.ack(&ping).unwrap();

    struct Cut<'a>(&'a Rc<Cell<bool>>);
    impl Model for Cut<'_> {
        fn reply(&self, _: &Prompt) -> Result<String, ModelError> {
            self.0.set(true);
            Ok("pong".into())
        }
    }
    let mut cut = world.klodik(Cut(&world.down));
    let step = cut.step().unwrap();
    assert!(step.sync_error.is_some() || !step.failed.is_empty());
    assert!(
        step.failed.iter().any(|(_, e)| e.starts_with("send:")),
        "{step:?}"
    );
    assert!(matches!(
        cut.state.get(ping.as_str()),
        Some(Handled::PendingSend { text, .. }) if text == "pong"
    ));

    world.down.set(false);
    let mut klodik = world.klodik(fake.clone());
    assert_eq!(klodik.step().unwrap().replied.len(), 1);
    assert_eq!(fake.calls(), 0, "the kept reply was sent");
    assert_eq!(world.iva_inbox().len(), 1);
}

#[test]
fn relay_down_loses_nothing() {
    let relay = MemoryRelay::default();
    let world = World::new("relay-down", &relay);
    let fake = Fake::default();
    world.iva_says("ping");
    world.klodik(Fake::answering(vec![])).shell.sync().unwrap();

    world.down.set(true);
    let mut klodik = world.klodik(fake.clone());
    let step = klodik.step().unwrap();
    assert!(step.sync_error.is_some());
    assert!(step.failed[0].1.starts_with("ack:"), "{step:?}");
    assert_eq!(fake.calls(), 0, "no model call it could not answer");

    world.down.set(false);
    assert_eq!(world.klodik(fake.clone()).step().unwrap().replied.len(), 1);
    assert_eq!(fake.calls(), 1);
}

// --- The real subprocess path, against harmless fake executables. ---

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn cli(program: PathBuf, dir: &Path) -> ClaudeCli {
    ClaudeCli {
        program,
        model: None,
        timeout: Duration::from_secs(2),
        workdir: dir.join("empty"),
    }
}

#[test]
fn subprocess_reply_is_the_result_not_stderr_noise() {
    let dir = dir("cli-ok");
    let args = dir.join("args");
    let stdin = dir.join("stdin");
    let program = script(
        &dir,
        "model",
        &format!(
            "for a in \"$@\"; do printf '[%s]\\n' \"$a\"; done > {}\ncat > {}\n\
             echo 'warning: noisy log line' >&2\n\
             printf '%s' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"pong\"}}'",
            args.display(),
            stdin.display()
        ),
    );
    let reply = cli(program, &dir)
        .reply(&Prompt::new(IVA, "ping klodik"))
        .unwrap();
    assert_eq!(reply, "pong");
    let args = std::fs::read_to_string(args).unwrap();
    assert!(args.contains("[--tools]\n[]\n"), "tools disabled: {args}");
    assert!(args.contains("[--safe-mode]"));
    assert!(!args.contains("ping klodik"));
    assert!(
        std::fs::read_to_string(stdin)
            .unwrap()
            .contains("ping klodik")
    );
}

#[test]
fn subprocess_failures_are_errors_not_replies() {
    let dir = dir("cli-fail");
    let prompt = Prompt::new(IVA, "ping");
    let failed = script(&dir, "failed", "echo 'auth: not logged in' >&2; exit 3");
    assert!(matches!(
        cli(failed, &dir).reply(&prompt),
        Err(ModelError::Failed { code: Some(3), stderr }) if stderr.contains("not logged in")
    ));
    let garbage = script(&dir, "garbage", "echo pong");
    assert!(matches!(
        cli(garbage, &dir).reply(&prompt),
        Err(ModelError::Malformed(_))
    ));
    let slow = script(&dir, "slow", "sleep 10");
    assert!(matches!(
        cli(slow, &dir).reply(&prompt),
        Err(ModelError::TimedOut(_))
    ));
    assert!(matches!(
        cli(dir.join("missing"), &dir).reply(&prompt),
        Err(ModelError::Spawn(_))
    ));
}

#[test]
fn full_flow_through_a_fake_executable() {
    let relay = MemoryRelay::default();
    let world = World::new("cli-flow", &relay);
    let program = script(
        &world.root,
        "model",
        "cat >/dev/null; printf '%s' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"pong from script\"}'",
    );
    let mut klodik = world.klodik(cli(program, &world.root));
    world.iva_says("ping klodik");
    assert_eq!(klodik.step().unwrap().replied.len(), 1);
    assert_eq!(world.iva_inbox()[0].body(), "pong from script");
}
