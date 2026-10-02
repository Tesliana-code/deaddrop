//! Node shell lifecycle over the in-process `MemoryRelay`: identity, explicit
//! peers, signed send, verified asynchronous inbox, and protocol ACK.

use std::path::{Path, PathBuf};

use deaddrop_protocol::{
    CorrelationId, DeliveryEventKind, EnvelopeV0, MessageId, MessageKind, NodeId,
};
use deaddrop_shell::{
    Acknowledged, LocalKey, MemoryRelay, PeerOutcome, Rejection, Relay, Shell, ShellError, init,
};

const A: &str = "node-a:shell:deaddrop";
const B: &str = "node-b:shell:deaddrop";
const C: &str = "node-c:shell:deaddrop";
const RELAY: &str = "http://127.0.0.1:8787";

fn node(value: &str) -> NodeId {
    NodeId::parse(value).unwrap()
}

fn home(test: &str, who: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-shell-tests")
        .join(test)
        .join(who);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    dir
}

fn open<'r>(dir: &Path, relay: &'r MemoryRelay) -> Shell<&'r MemoryRelay> {
    Shell::open_with(dir, relay).unwrap()
}

/// Initialise named nodes and make each trust the listed peers.
fn network<'r>(
    test: &str,
    relay: &'r MemoryRelay,
    nodes: &[&str],
    trust: &[(&str, &str)],
) -> Vec<Shell<&'r MemoryRelay>> {
    let shells: Vec<_> = nodes
        .iter()
        .map(|name| {
            let dir = home(test, name);
            init(&dir, node(name), RELAY).unwrap();
            open(&dir, relay)
        })
        .collect();
    for (truster, trusted) in trust {
        let card = shells[nodes.iter().position(|n| n == trusted).unwrap()].identity();
        let truster = &shells[nodes.iter().position(|n| n == truster).unwrap()];
        truster.add_peer(&card.node, &card.key).unwrap();
    }
    shells
}

fn kinds(events: &[deaddrop_protocol::DeliveryEvent]) -> Vec<DeliveryEventKind> {
    events.iter().map(|e| e.kind()).collect()
}

#[test]
fn init_creates_identity_with_private_secret_key() {
    let dir = home("init", "a");
    let card = init(&dir, node(A), RELAY).unwrap();
    assert_eq!(card.node, node(A));
    assert_eq!(card.relay, RELAY);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("secret.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let relay = MemoryRelay::default();
    assert_eq!(open(&dir, &relay).identity(), card);
    assert!(matches!(
        init(&dir, node(A), RELAY),
        Err(ShellError::AlreadyInitialized)
    ));
}

#[test]
fn init_refuses_non_loopback_relay_until_encryption_exists() {
    for relay in [
        "http://relay.example.org:8787",
        "http://10.0.0.5:8787",
        "https://203.0.113.9",
    ] {
        let dir = home("non-loopback", "a");
        assert!(
            matches!(
                init(&dir, node(A), relay),
                Err(ShellError::NonLoopbackRelay { .. })
            ),
            "{relay}"
        );
    }
    for relay in ["http://127.0.0.1:1", "http://localhost:2", "http://[::1]:3"] {
        let dir = home("loopback", "a");
        init(&dir, node(A), relay).unwrap();
    }
}

#[test]
fn peers_are_explicit_and_keys_do_not_silently_change() {
    let relay = MemoryRelay::default();
    let shells = network("peers", &relay, &[A, B, C], &[]);
    let (a, b, c) = (&shells[0], shells[1].identity(), shells[2].identity());

    assert!(a.peers().unwrap().is_empty());
    assert_eq!(a.add_peer(&b.node, &b.key).unwrap(), PeerOutcome::Added);
    assert_eq!(
        a.add_peer(&b.node, &b.key).unwrap(),
        PeerOutcome::AlreadyPresent
    );
    assert!(matches!(
        a.add_peer(&b.node, &c.key),
        Err(ShellError::PeerKeyConflict { .. })
    ));
    assert_eq!(a.peers().unwrap(), vec![(b.node, b.key)]);
}

#[test]
fn send_requires_a_trusted_peer() {
    let relay = MemoryRelay::default();
    let shells = network("send-untrusted", &relay, &[A, B], &[]);
    assert!(matches!(
        shells[0].send(&node(B), "hello", None, &[]),
        Err(ShellError::UnknownPeer { .. })
    ));
    assert!(relay.list_for(&node(B)).unwrap().is_empty());
}

#[test]
fn message_is_received_asynchronously_verified_and_idempotent() {
    let relay = MemoryRelay::default();
    let shells = network("receive", &relay, &[A, B], &[(A, B), (B, A)]);
    let (a, b) = (&shells[0], &shells[1]);

    let correlation = CorrelationId::parse("conversation-1").unwrap();
    let sent = a
        .send(&node(B), "hello b", Some(correlation.clone()), &[])
        .unwrap();
    // Nothing is delivered until B looks.
    assert!(b.inbox().unwrap().is_empty());

    let report = b.sync().unwrap();
    assert_eq!(report.received, vec![sent.clone()]);
    assert!(report.rejected.is_empty());
    let inbox = b.inbox().unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].id(), &sent);
    assert_eq!(inbox[0].from(), &node(A));
    assert_eq!(inbox[0].body(), "hello b");
    assert_eq!(inbox[0].correlation_id(), Some(&correlation));
    assert_eq!(
        kinds(&b.delivery(&sent).unwrap()),
        vec![
            DeliveryEventKind::RecipientReceived,
            DeliveryEventKind::RecipientVerified
        ]
    );

    let again = b.sync().unwrap();
    assert!(again.received.is_empty());
    assert!(again.rejected.is_empty());
    assert_eq!(b.inbox().unwrap().len(), 1);
}

#[test]
fn ack_round_trip_records_only_protocol_acknowledgment() {
    let relay = MemoryRelay::default();
    let shells = network("ack", &relay, &[A, B], &[(A, B), (B, A)]);
    let (a, b) = (&shells[0], &shells[1]);

    let sent = a.send(&node(B), "please confirm", None, &[]).unwrap();
    b.sync().unwrap();
    let ack = b.ack(&sent).unwrap();
    // Exact replay of the ACK is idempotent.
    assert_eq!(b.ack(&sent).unwrap(), ack);

    let report = a.sync().unwrap();
    assert_eq!(
        report.acknowledged,
        vec![Acknowledged {
            ack: ack.clone(),
            message: sent.clone(),
            by: node(B),
        }]
    );
    assert!(report.received.is_empty(), "an ACK is not an inbox message");
    assert!(a.inbox().unwrap().is_empty());
    let events = a.delivery(&sent).unwrap();
    assert_eq!(
        kinds(&events),
        vec![DeliveryEventKind::RecipientAcknowledged]
    );
    assert_eq!(events[0].reported_by(), &node(B));

    assert!(a.sync().unwrap().acknowledged.is_empty());
}

#[test]
fn only_received_peer_messages_can_be_acknowledged() {
    let relay = MemoryRelay::default();
    let shells = network("ack-rules", &relay, &[A, B], &[(A, B), (B, A)]);
    let (a, b) = (&shells[0], &shells[1]);
    let sent = a.send(&node(B), "hi", None, &[]).unwrap();

    assert!(matches!(
        b.ack(&sent),
        Err(ShellError::NotAcknowledgeable { .. })
    ));
    assert!(matches!(
        a.ack(&sent),
        Err(ShellError::NotAcknowledgeable { .. })
    ));
    assert!(matches!(
        a.ack(&MessageId::parse("never-seen").unwrap()),
        Err(ShellError::NotAcknowledgeable { .. })
    ));
}

/// Post a correctly signed envelope straight to the relay.
fn inject(relay: &MemoryRelay, key: &LocalKey, envelope: &EnvelopeV0) {
    relay
        .post_signature(&key.sign(envelope.from(), envelope))
        .unwrap();
    relay.post_message(envelope).unwrap();
}

fn key_of(dir: &Path) -> LocalKey {
    LocalKey::load(&dir.join("secret.key")).unwrap()
}

#[test]
fn ack_must_correlate_to_a_message_sent_to_that_peer() {
    let relay = MemoryRelay::default();
    let shells = network(
        "ack-correlation",
        &relay,
        &[A, B, C],
        &[(A, B), (A, C), (B, A), (C, A)],
    );
    let (a, b) = (&shells[0], &shells[1]);
    let sent = a.send(&node(B), "for b only", None, &[]).unwrap();
    b.sync().unwrap();

    let ack = |id: &str, by: &str, correlation: &str| {
        EnvelopeV0::new(
            MessageId::parse(id).unwrap(),
            node(by),
            node(A),
            MessageKind::Acknowledgment,
            "",
        )
        .with_correlation_id(CorrelationId::parse(correlation).unwrap())
    };
    // B acknowledges something A never sent.
    let unknown = ack("ack-unknown", B, "not-a-message");
    inject(&relay, &key_of(&home_path("ack-correlation", B)), &unknown);
    // C acknowledges A's message to B.
    let foreign = ack("ack-foreign", C, sent.as_str());
    inject(&relay, &key_of(&home_path("ack-correlation", C)), &foreign);
    // An ACK without any correlation.
    let bare = EnvelopeV0::new(
        MessageId::parse("ack-bare").unwrap(),
        node(B),
        node(A),
        MessageKind::Acknowledgment,
        "",
    );
    inject(&relay, &key_of(&home_path("ack-correlation", B)), &bare);

    let report = a.sync().unwrap();
    assert!(report.acknowledged.is_empty());
    let mut rejected = report.rejected.clone();
    rejected.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        rejected,
        vec![
            (bare.id().clone(), Rejection::UnmatchedAcknowledgment),
            (foreign.id().clone(), Rejection::UnmatchedAcknowledgment),
            (unknown.id().clone(), Rejection::UnmatchedAcknowledgment),
        ]
    );
    assert!(a.delivery(&sent).unwrap().is_empty());
}

fn home_path(test: &str, who: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("deaddrop-shell-tests")
        .join(test)
        .join(who)
}

#[test]
fn untrusted_or_forged_senders_are_rejected_and_not_stored() {
    let relay = MemoryRelay::default();
    // B trusts A only. C knows B's key and tries both honestly and forging A.
    let shells = network("untrusted", &relay, &[A, B, C], &[(B, A), (C, B)]);
    let (b, c) = (&shells[1], &shells[2]);

    let honest = c.send(&node(B), "hi from c", None, &[]).unwrap();
    let forged = EnvelopeV0::new(
        MessageId::parse("forged-1").unwrap(),
        node(A),
        node(B),
        MessageKind::Message,
        "definitely from a",
    );
    inject(&relay, &key_of(&home_path("untrusted", C)), &forged);

    let report = b.sync().unwrap();
    assert!(report.received.is_empty());
    let mut rejected = report.rejected.clone();
    rejected.sort_by(|x, y| x.0.cmp(&y.0));
    let mut expected = vec![
        (honest, Rejection::UntrustedSender),
        (forged.id().clone(), Rejection::NoTrustedSignature),
    ];
    expected.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(rejected, expected);
    assert!(b.inbox().unwrap().is_empty());
}

#[test]
fn extra_signature_records_on_the_relay_do_not_block_delivery() {
    let relay = MemoryRelay::default();
    let shells = network("noise", &relay, &[A, B, C], &[(A, B), (B, A)]);
    let (a, b) = (&shells[0], &shells[1]);
    let sent = a.send(&node(B), "real", None, &[]).unwrap();
    let envelope = relay.get_message(&sent).unwrap().unwrap();
    relay
        .post_signature(&key_of(&home_path("noise", C)).sign(&node(A), &envelope))
        .unwrap();

    assert_eq!(b.sync().unwrap().received, vec![sent]);
}

#[test]
fn artifacts_travel_by_reference_and_are_hash_checked() {
    let relay = MemoryRelay::default();
    let shells = network("artifact", &relay, &[A, B], &[(A, B), (B, A)]);
    let (a, b) = (&shells[0], &shells[1]);

    let artifact = a.put_artifact(b"immutable evidence").unwrap();
    let sent = a
        .send(
            &node(B),
            "see artifact",
            None,
            std::slice::from_ref(&artifact),
        )
        .unwrap();
    b.sync().unwrap();
    let received = &b.inbox().unwrap()[0];
    assert_eq!(received.id(), &sent);
    assert_eq!(received.artifact_refs(), std::slice::from_ref(&artifact));
    assert_eq!(b.get_artifact(&artifact).unwrap(), b"immutable evidence");
}
