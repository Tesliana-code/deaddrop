//! Envelope V0 conformance corpus regression.
//!
//! `protocol/vectors/envelope-v0.json` is the independent, language-neutral
//! expected contract (docs/PROTOCOL.md R1-R7). Wire bytes and field values
//! are hex-encoded there so they carry exact bytes. Any drift in the
//! production encoder or decoder, including a serde_json upgrade that changes
//! canonical output, fails this test.

use deaddrop_protocol::{decode_envelope_v0, encode_envelope_v0};
use serde_json::Value;

const CORPUS: &str = include_str!("../../../protocol/vectors/envelope-v0.json");

const POSITIVE: usize = 29;
const NEGATIVE: usize = 60;

fn unhex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).expect("corpus hex"))
        .collect()
}

fn text(value: &Value) -> String {
    String::from_utf8(unhex(value.as_str().expect("hex string"))).expect("utf-8 field")
}

fn vectors() -> Vec<Value> {
    let corpus: Value = serde_json::from_str(CORPUS).expect("corpus parses");
    assert_eq!(corpus["corpus"], "deaddrop-envelope-v0-wire-conformance");
    assert_eq!(corpus["revision"], 1);
    corpus["vectors"].as_array().expect("vectors").clone()
}

fn of_class(class: &str) -> Vec<Value> {
    vectors()
        .into_iter()
        .filter(|vector| vector["class"] == class)
        .collect()
}

#[test]
fn corpus_has_expected_shape() {
    let all = vectors();
    assert_eq!(all.len(), POSITIVE + NEGATIVE);
    assert_eq!(of_class("positive").len(), POSITIVE);
    assert_eq!(of_class("negative").len(), NEGATIVE);
}

#[test]
fn positive_vectors_decode_to_corpus_fields_and_reencode_exactly() {
    let positives = of_class("positive");
    assert_eq!(positives.len(), POSITIVE);

    for vector in &positives {
        let id = vector["id"].as_str().unwrap();
        let wire = unhex(vector["wire_hex"].as_str().unwrap());
        let fields = &vector["fields_utf8_hex"];

        let envelope = decode_envelope_v0(&wire)
            .unwrap_or_else(|error| panic!("{id}: rejected positive vector: {error}"));

        assert_eq!(envelope.protocol(), "deaddrop/0", "{id}");
        assert_eq!(envelope.id().as_str(), text(&fields["id"]), "{id}: id");
        assert_eq!(
            envelope.from().as_str(),
            text(&fields["from"]),
            "{id}: from"
        );
        assert_eq!(envelope.to().as_str(), text(&fields["to"]), "{id}: to");
        assert_eq!(
            envelope.kind().as_str(),
            text(&fields["kind"]),
            "{id}: kind"
        );
        assert_eq!(
            envelope
                .correlation_id()
                .map(|value| value.as_str().to_owned()),
            fields["correlation_id"]
                .as_str()
                .map(|_| text(&fields["correlation_id"])),
            "{id}: correlation_id"
        );
        assert_eq!(envelope.body(), text(&fields["body"]), "{id}: body");

        let refs: Vec<String> = envelope
            .artifact_refs()
            .iter()
            .map(ToString::to_string)
            .collect();
        let expected_refs: Vec<String> = fields["artifact_refs"]
            .as_array()
            .unwrap()
            .iter()
            .map(text)
            .collect();
        assert_eq!(refs, expected_refs, "{id}: artifact_refs");

        let encoded = encode_envelope_v0(&envelope).unwrap();
        assert!(
            encoded == wire,
            "{id}: canonical re-encode drifted\n expected {}\n   actual {}",
            String::from_utf8_lossy(&wire),
            String::from_utf8_lossy(&encoded)
        );
    }
}

#[test]
fn negative_vectors_are_rejected() {
    let negatives = of_class("negative");
    assert_eq!(negatives.len(), NEGATIVE);

    for vector in &negatives {
        let id = vector["id"].as_str().unwrap();
        let wire = unhex(vector["wire_hex"].as_str().unwrap());

        assert!(
            decode_envelope_v0(&wire).is_err(),
            "{id}: accepted negative vector ({})",
            vector["note"]
        );
    }
}
