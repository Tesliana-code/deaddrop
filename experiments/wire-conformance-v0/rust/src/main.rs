//! Reference observation: runs the production deaddrop-protocol V0 wire
//! encoder/decoder over the language-neutral corpus.
//!
//! Emits one JSON line per vector and per escape probe on stdout.

use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, decode_envelope_v0,
    encode_envelope_v0,
};
use serde_json::{Value, json};

const IMPL: &str = "rust-reference";

fn unhex(value: &str) -> Vec<u8> {
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).expect("corpus hex"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn text(value: &Value) -> String {
    String::from_utf8(unhex(value.as_str().expect("hex string"))).expect("utf-8 field")
}

struct Fields {
    id: String,
    from: String,
    to: String,
    kind: String,
    correlation_id: Option<String>,
    body: String,
    artifact_refs: Vec<String>,
}

fn fields_from_hex(value: &Value) -> Fields {
    Fields {
        id: text(&value["id"]),
        from: text(&value["from"]),
        to: text(&value["to"]),
        kind: text(&value["kind"]),
        correlation_id: value["correlation_id"]
            .as_str()
            .map(|_| text(&value["correlation_id"])),
        body: text(&value["body"]),
        artifact_refs: value["artifact_refs"]
            .as_array()
            .expect("refs")
            .iter()
            .map(text)
            .collect(),
    }
}

fn build(fields: &Fields) -> Result<EnvelopeV0, String> {
    let kind = MessageKind::parse(&fields.kind).ok_or("unknown kind")?;
    let mut envelope = EnvelopeV0::new(
        MessageId::parse(fields.id.clone()).map_err(|e| e.to_string())?,
        NodeId::parse(fields.from.clone()).map_err(|e| e.to_string())?,
        NodeId::parse(fields.to.clone()).map_err(|e| e.to_string())?,
        kind,
        fields.body.clone(),
    );
    if let Some(correlation_id) = &fields.correlation_id {
        envelope = envelope.with_correlation_id(
            CorrelationId::parse(correlation_id.clone()).map_err(|e| e.to_string())?,
        );
    }
    for artifact_ref in &fields.artifact_refs {
        envelope = envelope
            .with_artifact_ref(ArtifactRef::from_str(artifact_ref).map_err(|e| e.to_string())?);
    }
    Ok(envelope)
}

fn same(envelope: &EnvelopeV0, fields: &Fields) -> bool {
    envelope.id().as_str() == fields.id
        && envelope.from().as_str() == fields.from
        && envelope.to().as_str() == fields.to
        && envelope.kind().as_str() == fields.kind
        && envelope.correlation_id().map(|c| c.as_str().to_owned()) == fields.correlation_id
        && envelope.body() == fields.body
        && envelope
            .artifact_refs()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            == fields.artifact_refs
}

fn body_literal(wire: &[u8]) -> Vec<u8> {
    let text = wire;
    let start = find(text, b"\"body\":").expect("body member") + 7;
    let end = find(text, b",\"artifact_refs\"").expect("refs member");
    text[start..end].to_vec()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: <corpus.json>");
    let corpus: Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read corpus")).expect("corpus");

    for vector in corpus["vectors"].as_array().expect("vectors") {
        let id = vector["id"].as_str().unwrap();
        let class = vector["class"].as_str().unwrap();
        let wire = unhex(vector["wire_hex"].as_str().unwrap());

        let (encode, encoded_hex, expected) = if class == "positive" {
            let fields = fields_from_hex(&vector["fields_utf8_hex"]);
            match build(&fields)
                .map_err(|e| e.to_string())
                .and_then(|envelope| encode_envelope_v0(&envelope).map_err(|e| e.to_string()))
            {
                Ok(bytes) if bytes == wire => ("match", Value::Null, Some(fields)),
                Ok(bytes) => ("mismatch", json!(hex(&bytes)), Some(fields)),
                Err(error) => ("error", json!(error), Some(fields)),
            }
        } else {
            ("n/a", Value::Null, None)
        };

        let (decode, fields_match, detail) = match decode_envelope_v0(&wire) {
            Ok(envelope) => (
                "accept",
                expected.as_ref().map(|f| same(&envelope, f)),
                String::new(),
            ),
            Err(error) => ("reject", None, error.to_string()),
        };

        let pass = if class == "positive" {
            encode == "match" && decode == "accept" && fields_match == Some(true)
        } else {
            decode == "reject"
        };

        println!(
            "{}",
            json!({
                "type": "vector", "impl": IMPL, "vector": id, "class": class,
                "encode": encode, "encoded_hex": encoded_hex, "decode": decode,
                "fields_match": fields_match, "detail": detail, "pass": pass,
            })
        );
    }

    let golden = &corpus["golden_fields"];
    for codepoint in corpus["escape_probe_codepoints"].as_array().unwrap() {
        let cp = codepoint.as_u64().unwrap() as u32;
        let body = char::from_u32(cp).unwrap().to_string();
        let envelope = EnvelopeV0::new(
            MessageId::parse(golden["id"].as_str().unwrap()).unwrap(),
            NodeId::parse(golden["from"].as_str().unwrap()).unwrap(),
            NodeId::parse(golden["to"].as_str().unwrap()).unwrap(),
            MessageKind::Message,
            body,
        );
        let wire = encode_envelope_v0(&envelope).unwrap();
        println!(
            "{}",
            json!({"type": "escape", "impl": IMPL, "codepoint": cp,
                   "literal_hex": hex(&body_literal(&wire)), "error": null})
        );
    }
}
