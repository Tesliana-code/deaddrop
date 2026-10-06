//! Klodik's conversational context through the real peer loop: real
//! `Shell`s on an in-process relay, a fake model that records every prompt.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use deaddrop_klodik::context::{Conversational, MAX_EXCHANGES, Transcripts};
use deaddrop_klodik::model::{Model, ModelError, PERSONA, Prompt};
use deaddrop_klodik::{Handled, Klodik, State};
use deaddrop_protocol::{ArtifactRef, EnvelopeV0, MessageId, NodeId, SignatureV0};
use deaddrop_shell::{MemoryRelay, Relay, RelayError, Shell, init};

const IVA: &str = "iva:local:deaddrop";
const DANIL: &str = "danil:local:deaddrop";
const KLODIK: &str = "klodik:agent:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(id: &str) -> NodeId {
    NodeId::parse(id).unwrap()
}

/// A relay that can be switched off.
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

/// Records prompts; answers from a script, else echoes a fixed reply.
#[derive(Clone, Default)]
struct Recorder {
    prompts: Rc<RefCell<Vec<Prompt>>>,
    script: Rc<RefCell<Vec<Result<String, ModelError>>>>,
    /// Switch the relay off as the model answers, to fail the send.
    cut_relay: Option<Rc<Cell<bool>>>,
}

impl Recorder {
    fn last(&self) -> String {
        self.prompts.borrow().last().unwrap().user.clone()
    }
    fn calls(&self) -> usize {
        self.prompts.borrow().len()
    }
}

impl Model for Recorder {
    fn reply(&self, prompt: &Prompt) -> Result<String, ModelError> {
        self.prompts.borrow_mut().push(prompt.clone());
        if let Some(down) = &self.cut_relay {
            down.set(true);
        }
        let mut script = self.script.borrow_mut();
        if script.is_empty() {
            Ok(format!("reply {}", self.prompts.borrow().len()))
        } else {
            script.remove(0)
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
    fn new(test: &str, relay: &'r MemoryRelay) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("deaddrop-klodik-context-tests")
            .join(test);
        let _ = std::fs::remove_dir_all(&root);
        let open = |who: &str| {
            let home = root.join(who);
            init(&home, node(who), RELAY).unwrap();
            Shell::open_with(&home, relay).unwrap()
        };
        let (iva, danil, klodik) = (open(IVA), open(DANIL), open(KLODIK));
        let k = klodik.identity();
        for human in [&iva, &danil] {
            let h = human.identity();
            human.add_peer(&k.node, &k.key).unwrap();
            klodik.add_peer(&h.node, &h.key).unwrap();
        }
        Self {
            root,
            relay,
            down: Rc::new(Cell::new(false)),
            iva,
            danil,
        }
    }

    fn transcripts_path(&self) -> PathBuf {
        self.root.join(KLODIK).join("klodik/transcripts.json")
    }

    /// A Klodik process; building another over the same home is a restart.
    fn klodik(&self, model: Recorder) -> Klodik<Switch<'r>, Conversational<Recorder>> {
        let home = self.root.join(KLODIK);
        Klodik {
            shell: Shell::open_with(
                &home,
                Switch {
                    relay: self.relay,
                    down: self.down.clone(),
                },
            )
            .unwrap(),
            model: Conversational {
                model,
                transcripts: Transcripts::load(&self.transcripts_path()).unwrap(),
                own: KLODIK.into(),
            },
            state: State::load(&home.join("klodik/state.json")).unwrap(),
            allowed: vec![node(IVA), node(DANIL)],
            rooms: Vec::new(),
            may_ask: Vec::new(),
        }
    }

    fn say(&self, from: &Shell<&'r MemoryRelay>, body: &str) -> MessageId {
        from.send(&node(KLODIK), body, None, &[]).unwrap()
    }
}

#[test]
fn a_conversation_carries_forward_one_exchange_at_a_time() {
    let relay = MemoryRelay::default();
    let world = World::new("carry", &relay);
    let model = Recorder::default();
    *model.script.borrow_mut() = vec![Ok("Hobotnica je sjajna. Moja je vidra.".into())];
    let mut klodik = world.klodik(model.clone());

    let first = world.say(&world.iva, "moja omiljena životinja je hobotnica");
    klodik.step().unwrap();
    assert_eq!(model.prompts.borrow()[0].system, PERSONA);
    assert_eq!(
        model.last(),
        "CURRENT MESSAGE FROM iva\n\n  moja omiljena životinja je hobotnica",
        "the first message has no transcript"
    );
    let kept = klodik.model.transcripts.exchanges(IVA).to_vec();
    assert_eq!(kept.len(), 1, "the completed exchange is kept");
    assert_eq!(kept[0].user.message_id, first.to_string());
    assert_eq!(
        kept[0].assistant.text,
        "Hobotnica je sjajna. Moja je vidra."
    );

    world.say(&world.iva, "a moja?");
    klodik.step().unwrap();
    assert_eq!(
        model.last(),
        "CONVERSATION SO FAR\n\n\
         iva:\n  moja omiljena životinja je hobotnica\n\n\
         klodik:\n  Hobotnica je sjajna. Moja je vidra.\n\n\
         CURRENT MESSAGE FROM iva\n\n  a moja?"
    );

    // "isto" means nothing alone; with the transcript the topic is there.
    world.say(&world.iva, "isto");
    klodik.step().unwrap();
    let prompt = model.last();
    assert!(prompt.contains("hobotnica") && prompt.contains("a moja?"));
    assert!(prompt.ends_with("CURRENT MESSAGE FROM iva\n\n  isto"));
}

#[test]
fn transcripts_are_kept_per_peer() {
    let relay = MemoryRelay::default();
    let world = World::new("per-peer", &relay);
    let model = Recorder::default();
    let mut klodik = world.klodik(model.clone());
    world.say(&world.iva, "iva's secret topic: hobotnica");
    klodik.step().unwrap();
    world.say(&world.danil, "zdravo");
    klodik.step().unwrap();
    assert_eq!(
        model.last(),
        "CURRENT MESSAGE FROM danil\n\n  zdravo",
        "Danil does not see Iva's conversation"
    );
}

#[test]
fn a_failed_model_call_adds_nothing_to_the_transcript() {
    let relay = MemoryRelay::default();
    let world = World::new("model-fails", &relay);
    let model = Recorder::default();
    *model.script.borrow_mut() = vec![Err(ModelError::Empty)];
    let mut klodik = world.klodik(model.clone());
    world.say(&world.iva, "hello?");
    klodik.step().unwrap();
    assert!(klodik.model.transcripts.exchanges(IVA).is_empty());

    // The retry succeeds; still only the one completed exchange, and the
    // failed attempt left no assistant turn behind.
    klodik.step().unwrap();
    let kept = klodik.model.transcripts.exchanges(IVA);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].assistant.text, "reply 2");
}

#[test]
fn a_failed_send_is_retried_without_the_model_and_recorded_once() {
    let relay = MemoryRelay::default();
    let world = World::new("send-fails", &relay);
    let asked = world.say(&world.iva, "ping");
    // ACK while the relay is up; the model then takes the relay down.
    let model = Recorder {
        cut_relay: Some(world.down.clone()),
        ..Recorder::default()
    };
    let mut klodik = world.klodik(model.clone());
    let step = klodik.step().unwrap();
    assert!(
        step.failed.iter().any(|(_, e)| e.starts_with("send:")),
        "{step:?}"
    );
    assert!(matches!(
        klodik.state.get(asked.as_str()),
        Some(Handled::PendingSend { .. })
    ));
    assert!(
        klodik.model.transcripts.exchanges(IVA).is_empty(),
        "not sent, not context"
    );

    world.down.set(false);
    let later = Recorder::default();
    let mut klodik = world.klodik(later.clone());
    assert_eq!(klodik.step().unwrap().replied.len(), 1);
    assert_eq!(model.calls() + later.calls(), 1, "the model ran once");
    let kept = klodik.model.transcripts.exchanges(IVA);
    assert_eq!(kept.len(), 1);
    assert_eq!(
        kept[0].assistant.text, "reply 1",
        "the kept reply is what was sent"
    );
}

#[test]
fn the_transcript_survives_a_restart() {
    let relay = MemoryRelay::default();
    let world = World::new("restart", &relay);
    world.say(&world.iva, "zapamti: hobotnica");
    world.klodik(Recorder::default()).step().unwrap();

    let model = Recorder::default();
    let mut klodik = world.klodik(model.clone());
    world.say(&world.iva, "šta sam rekla?");
    klodik.step().unwrap();
    assert!(
        model
            .last()
            .starts_with("CONVERSATION SO FAR\n\niva:\n  zapamti: hobotnica\n")
    );
}

#[test]
fn only_the_newest_exchanges_are_kept() {
    let relay = MemoryRelay::default();
    let world = World::new("eight", &relay);
    let model = Recorder::default();
    let mut klodik = world.klodik(model.clone());
    for n in 1..=10 {
        world.say(&world.iva, &format!("message {n}."));
        klodik.step().unwrap();
    }
    let kept: Vec<String> = klodik
        .model
        .transcripts
        .exchanges(IVA)
        .iter()
        .map(|e| e.user.text.clone())
        .collect();
    assert_eq!(kept.len(), MAX_EXCHANGES);
    assert_eq!(kept.first().unwrap(), "message 3.");
    world.say(&world.iva, "next");
    klodik.step().unwrap();
    let prompt = model.last();
    assert!(!prompt.contains("message 2.") && prompt.contains("message 3."));
}

#[test]
fn replay_and_restart_still_answer_once() {
    let relay = MemoryRelay::default();
    let world = World::new("replay", &relay);
    let model = Recorder::default();
    world.say(&world.iva, "ping");
    let mut klodik = world.klodik(model.clone());
    klodik.step().unwrap();
    klodik.step().unwrap();
    world.klodik(model.clone()).step().unwrap();
    assert_eq!(model.calls(), 1);
    assert_eq!(
        Transcripts::load(&world.transcripts_path())
            .unwrap()
            .exchanges(IVA)
            .len(),
        1
    );
    world.iva.sync().unwrap();
    assert_eq!(world.iva.inbox().unwrap().len(), 1);
}
