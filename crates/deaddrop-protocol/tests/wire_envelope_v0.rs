use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, WireEnvelopeError,
    decode_envelope_v0, encode_envelope_v0,
};

fn envelope() -> EnvelopeV0 {
    EnvelopeV0::new(
        MessageId::parse("msg-wire-1").unwrap(),
        NodeId::parse("node-a:agent:deaddrop").unwrap(),
        NodeId::parse("node-b:agent:deaddrop").unwrap(),
        MessageKind::Handoff,
        "continue this task",
    )
    .with_correlation_id(CorrelationId::parse("corr-wire-1").unwrap())
    .with_artifact_ref(
        ArtifactRef::from_str(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .unwrap(),
    )
}

#[test]
fn canonical_wire_bytes_are_exact_and_stable() {
    let encoded = encode_envelope_v0(&envelope()).unwrap();

    let expected = concat!(
        r#"{"protocol":"deaddrop/0","id":"msg-wire-1","#,
        r#""from":"node-a:agent:deaddrop","#,
        r#""to":"node-b:agent:deaddrop","#,
        r#""kind":"handoff","#,
        r#""correlation_id":"corr-wire-1","#,
        r#""body":"continue this task","#,
        r#""artifact_refs":["#,
        r#""sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#
    );

    assert_eq!(encoded, expected.as_bytes());
}

#[test]
fn canonical_wire_roundtrip_preserves_exact_envelope() {
    let original = envelope();

    let encoded = encode_envelope_v0(&original).unwrap();
    let decoded = decode_envelope_v0(&encoded).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(encode_envelope_v0(&decoded).unwrap(), encoded);
}

#[test]
fn absent_correlation_is_encoded_explicitly_as_null() {
    let envelope = EnvelopeV0::new(
        MessageId::parse("msg-no-corr").unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse("node-b").unwrap(),
        MessageKind::Message,
        "hello",
    );

    let encoded = encode_envelope_v0(&envelope).unwrap();
    let text = String::from_utf8(encoded).unwrap();

    assert!(text.contains(r#""correlation_id":null"#));
}

#[test]
fn noncanonical_json_representation_is_rejected() {
    let canonical = encode_envelope_v0(&envelope()).unwrap();
    let mut pretty: serde_json::Value = serde_json::from_slice(&canonical).unwrap();

    pretty["body"] = serde_json::Value::String("continue this task".to_owned());

    let noncanonical = serde_json::to_vec_pretty(&pretty).unwrap();

    let result = decode_envelope_v0(&noncanonical);

    assert!(matches!(
        result,
        Err(WireEnvelopeError::NonCanonicalEncoding)
    ));
}

#[test]
fn wrong_protocol_is_rejected() {
    let bytes = br#"{"protocol":"deaddrop/9","id":"msg-wire-1","from":"node-a:agent:deaddrop","to":"node-b:agent:deaddrop","kind":"handoff","correlation_id":"corr-wire-1","body":"continue this task","artifact_refs":["sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]}"#;

    let result = decode_envelope_v0(bytes);

    assert!(matches!(
        result,
        Err(WireEnvelopeError::UnsupportedProtocol { .. })
    ));
}

#[test]
fn unknown_message_kind_is_rejected() {
    let bytes = br#"{"protocol":"deaddrop/0","id":"msg-wire-1","from":"node-a:agent:deaddrop","to":"node-b:agent:deaddrop","kind":"mystery","correlation_id":"corr-wire-1","body":"continue this task","artifact_refs":[]}"#;

    let result = decode_envelope_v0(bytes);

    assert!(matches!(
        result,
        Err(WireEnvelopeError::UnknownMessageKind { .. })
    ));
}

#[test]
fn unknown_wire_field_is_rejected() {
    let bytes = br#"{"protocol":"deaddrop/0","id":"msg-wire-1","from":"node-a","to":"node-b","kind":"message","correlation_id":null,"body":"hello","artifact_refs":[],"surprise":"field"}"#;

    assert!(matches!(
        decode_envelope_v0(bytes),
        Err(WireEnvelopeError::Json(_))
    ));
}
