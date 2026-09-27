use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, PROTOCOL_V0,
};

fn node(value: &str) -> NodeId {
    NodeId::parse(value).expect("valid node id")
}

fn message_id(value: &str) -> MessageId {
    MessageId::parse(value).expect("valid message id")
}

#[test]
fn protocol_v0_identifier_is_stable() {
    assert_eq!(PROTOCOL_V0, "deaddrop/0");

    let envelope = EnvelopeV0::new(
        message_id("msg-contract-1"),
        node("node-a"),
        node("node-b"),
        MessageKind::Message,
        "hello",
    );

    assert_eq!(envelope.protocol(), PROTOCOL_V0);
}

#[test]
fn every_message_kind_roundtrips_through_protocol_name() {
    let kinds = [
        MessageKind::Message,
        MessageKind::Request,
        MessageKind::Response,
        MessageKind::Handoff,
        MessageKind::TaskClaim,
        MessageKind::Checkpoint,
        MessageKind::Acknowledgment,
        MessageKind::Error,
        MessageKind::CapabilityDeclaration,
    ];

    for kind in kinds {
        let encoded = kind.as_str();
        let decoded = MessageKind::parse(encoded);

        assert_eq!(decoded, Some(kind));
    }

    assert_eq!(MessageKind::parse("unknown_kind"), None);
}

#[test]
fn addressing_is_explicit_and_preserved() {
    let envelope = EnvelopeV0::new(
        message_id("msg-contract-2"),
        node("sender-node"),
        node("recipient-node"),
        MessageKind::Request,
        "perform bounded work",
    );

    assert_eq!(envelope.from().as_str(), "sender-node");
    assert_eq!(envelope.to().as_str(), "recipient-node");
}

#[test]
fn retransmission_preserves_semantic_message_identity() {
    let original = EnvelopeV0::new(
        message_id("msg-stable"),
        node("sender"),
        node("recipient"),
        MessageKind::Handoff,
        "continue this task",
    )
    .with_correlation_id(CorrelationId::parse("corr-stable").expect("valid correlation id"));

    let retransmitted = original.clone();

    assert_eq!(original.id(), retransmitted.id());
    assert_eq!(original, retransmitted);
}

#[test]
fn correlation_is_explicit_and_remains_part_of_the_envelope() {
    let envelope = EnvelopeV0::new(
        message_id("msg-contract-3"),
        node("sender"),
        node("recipient"),
        MessageKind::Response,
        "done",
    )
    .with_correlation_id(CorrelationId::parse("corr-42").expect("valid correlation id"));

    assert_eq!(
        envelope
            .correlation_id()
            .expect("correlation should exist")
            .as_str(),
        "corr-42"
    );
}

#[test]
fn artifact_references_preserve_exact_content_identity() {
    let first = ArtifactRef::from_str(
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .expect("valid artifact ref");

    let second = ArtifactRef::from_str(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("valid artifact ref");

    let envelope = EnvelopeV0::new(
        message_id("msg-contract-4"),
        node("sender"),
        node("recipient"),
        MessageKind::Handoff,
        "evidence by immutable reference",
    )
    .with_artifact_ref(first.clone())
    .with_artifact_ref(second.clone());

    assert_eq!(envelope.artifact_refs(), &[first, second]);
}

#[test]
fn body_and_kind_are_preserved_as_semantic_message_content() {
    let envelope = EnvelopeV0::new(
        message_id("msg-contract-5"),
        node("sender"),
        node("recipient"),
        MessageKind::TaskClaim,
        "claim task alpha",
    );

    assert_eq!(envelope.kind(), MessageKind::TaskClaim);
    assert_eq!(envelope.body(), "claim task alpha");
}
