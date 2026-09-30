//! R8 Envelope V0 limits: exact-limit acceptance and +1 rejection.

use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeLimitError, EnvelopeV0, IdError, MAX_ARTIFACT_REFS,
    MAX_CANONICAL_ENVELOPE_BYTES, MAX_IDENTIFIER_UTF8_BYTES, MessageId, MessageKind, NodeId,
    WireEnvelopeError, check_envelope_v0_limits, decode_envelope_v0, encode_envelope_v0,
};

fn envelope(body: &str) -> EnvelopeV0 {
    EnvelopeV0::new(
        MessageId::parse("msg-limits").unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse("node-b").unwrap(),
        MessageKind::Message,
        body,
    )
}

fn artifact(n: usize) -> ArtifactRef {
    ArtifactRef::from_str(&format!("sha256:{n:064x}")).unwrap()
}

fn with_refs(mut envelope: EnvelopeV0, refs: impl IntoIterator<Item = ArtifactRef>) -> EnvelopeV0 {
    for artifact_ref in refs {
        envelope = envelope.with_artifact_ref(artifact_ref);
    }
    envelope
}

/// Envelope whose canonical encoding is exactly `len` bytes.
fn envelope_of_len(len: usize) -> EnvelopeV0 {
    let framing = encode_envelope_v0(&envelope("")).unwrap().len();
    envelope(&"x".repeat(len - framing))
}

/// Canonical bytes of exactly `len` bytes, produced without the encoder's
/// limit check by splicing the body directly.
fn canonical_bytes_of_len(len: usize) -> Vec<u8> {
    let empty = String::from_utf8(encode_envelope_v0(&envelope("")).unwrap()).unwrap();
    let body = "x".repeat(len - empty.len());
    let bytes = empty
        .replace(r#""body":"""#, &format!(r#""body":"{body}""#))
        .into_bytes();
    assert_eq!(bytes.len(), len);
    bytes
}

fn assert_limit(
    result: Result<impl std::fmt::Debug, WireEnvelopeError>,
    expected: EnvelopeLimitError,
) {
    match result {
        Err(WireEnvelopeError::Limit(actual)) => assert_eq!(actual, expected),
        other => panic!("expected {expected:?}, got {other:?}"),
    }
}

// A
#[test]
fn a_envelope_of_exactly_max_bytes_is_accepted() {
    let bytes = canonical_bytes_of_len(MAX_CANONICAL_ENVELOPE_BYTES);

    let decoded = decode_envelope_v0(&bytes).expect("exact-limit envelope accepted");
    assert_eq!(encode_envelope_v0(&decoded).unwrap(), bytes);
    assert_eq!(check_envelope_v0_limits(&decoded), Ok(()));
}

// B
#[test]
fn b_envelope_of_max_plus_one_bytes_is_rejected() {
    let bytes = canonical_bytes_of_len(MAX_CANONICAL_ENVELOPE_BYTES + 1);

    assert_limit(
        decode_envelope_v0(&bytes),
        EnvelopeLimitError::EnvelopeTooLarge {
            len: MAX_CANONICAL_ENVELOPE_BYTES + 1,
        },
    );
}

// B: the size check runs before JSON parsing, so even non-JSON of that size
// reports the limit rather than a parse error.
#[test]
fn b_oversize_is_rejected_before_parse() {
    let garbage = vec![b'{'; MAX_CANONICAL_ENVELOPE_BYTES + 1];

    assert_limit(
        decode_envelope_v0(&garbage),
        EnvelopeLimitError::EnvelopeTooLarge {
            len: MAX_CANONICAL_ENVELOPE_BYTES + 1,
        },
    );

    let at_limit = vec![b'{'; MAX_CANONICAL_ENVELOPE_BYTES];
    assert!(matches!(
        decode_envelope_v0(&at_limit),
        Err(WireEnvelopeError::Json(_))
    ));
}

// C, D, E: every identifier field on the wire.
#[test]
fn c_d_e_identifier_limit_is_utf8_bytes_on_every_field() {
    let exact_ascii = "a".repeat(MAX_IDENTIFIER_UTF8_BYTES);
    // 64 four-byte scalars: 64 characters, 256 bytes.
    let exact_multibyte = "\u{1F600}".repeat(MAX_IDENTIFIER_UTF8_BYTES / 4);
    let over_ascii = "a".repeat(MAX_IDENTIFIER_UTF8_BYTES + 1);
    // 128 two-byte scalars plus one byte: 129 characters, 257 bytes.
    let over_multibyte = format!("{}a", "\u{e9}".repeat(MAX_IDENTIFIER_UTF8_BYTES / 2));
    assert_eq!(exact_multibyte.chars().count(), 64);
    assert_eq!(over_multibyte.chars().count(), 129);

    let base = String::from_utf8(
        encode_envelope_v0(
            &envelope("body").with_correlation_id(CorrelationId::parse("corr").unwrap()),
        )
        .unwrap(),
    )
    .unwrap();

    let slots = [
        ("id", r#""id":"msg-limits""#),
        ("from", r#""from":"node-a""#),
        ("to", r#""to":"node-b""#),
        ("correlation_id", r#""correlation_id":"corr""#),
    ];

    for (field, original) in slots {
        let wire = |value: &str| base.replace(original, &format!(r#""{field}":"{value}""#));

        for exact in [&exact_ascii, &exact_multibyte] {
            let bytes = wire(exact);
            let decoded = decode_envelope_v0(bytes.as_bytes())
                .unwrap_or_else(|error| panic!("{field} at limit rejected: {error}"));
            assert_eq!(encode_envelope_v0(&decoded).unwrap(), bytes.as_bytes());
        }

        for over in [&over_ascii, &over_multibyte] {
            match decode_envelope_v0(wire(over).as_bytes()) {
                Err(WireEnvelopeError::InvalidIdentifier {
                    field: rejected,
                    error: IdError::TooLong { len: 257 },
                    ..
                }) => assert_eq!(rejected, field),
                other => panic!("{field} over limit: {other:?}"),
            }
        }
    }
}

// C: the identifier limit counts the decoded string, not its escaped wire form.
#[test]
fn c_identifier_limit_ignores_wire_escaping() {
    let quotes = "\"".repeat(MAX_IDENTIFIER_UTF8_BYTES);
    let envelope = EnvelopeV0::new(
        MessageId::parse(quotes).unwrap(),
        NodeId::parse("node-a").unwrap(),
        NodeId::parse("node-b").unwrap(),
        MessageKind::Message,
        "",
    );

    let bytes = encode_envelope_v0(&envelope).unwrap();
    assert_eq!(decode_envelope_v0(&bytes).unwrap(), envelope);
}

// F
#[test]
fn f_max_artifact_refs_are_accepted() {
    let envelope = with_refs(envelope(""), (0..MAX_ARTIFACT_REFS).map(artifact));

    let bytes = encode_envelope_v0(&envelope).unwrap();
    assert_eq!(decode_envelope_v0(&bytes).unwrap(), envelope);
}

// G
#[test]
fn g_max_plus_one_artifact_refs_are_rejected() {
    let over = with_refs(envelope(""), (0..MAX_ARTIFACT_REFS + 1).map(artifact));
    let expected = EnvelopeLimitError::TooManyArtifactRefs {
        count: MAX_ARTIFACT_REFS + 1,
    };

    // Encoder refuses.
    assert_limit(encode_envelope_v0(&over), expected);
    assert_eq!(check_envelope_v0_limits(&over), Err(expected));

    // Decoder rejects otherwise-canonical bytes.
    let at_limit = String::from_utf8(
        encode_envelope_v0(&with_refs(
            envelope(""),
            (0..MAX_ARTIFACT_REFS).map(artifact),
        ))
        .unwrap(),
    )
    .unwrap();
    let wire = at_limit.replace(
        r#""artifact_refs":["#,
        &format!(r#""artifact_refs":["{}","#, artifact(MAX_ARTIFACT_REFS)),
    );
    assert!(wire.len() < MAX_CANONICAL_ENVELOPE_BYTES);
    assert_limit(decode_envelope_v0(wire.as_bytes()), expected);
}

// H
#[test]
fn h_max_identical_duplicate_refs_are_accepted_and_preserved() {
    let envelope = with_refs(envelope(""), (0..MAX_ARTIFACT_REFS).map(|_| artifact(7)));

    let bytes = encode_envelope_v0(&envelope).unwrap();
    let decoded = decode_envelope_v0(&bytes).unwrap();
    assert_eq!(decoded.artifact_refs().len(), MAX_ARTIFACT_REFS);
    assert_eq!(decoded, envelope);

    let over = with_refs(envelope, [artifact(7)]);
    assert_limit(
        encode_envelope_v0(&over),
        EnvelopeLimitError::TooManyArtifactRefs {
            count: MAX_ARTIFACT_REFS + 1,
        },
    );
}

// I
#[test]
fn i_encode_refuses_oversized_in_memory_envelope() {
    let exact = envelope_of_len(MAX_CANONICAL_ENVELOPE_BYTES);
    assert_eq!(
        encode_envelope_v0(&exact).unwrap().len(),
        MAX_CANONICAL_ENVELOPE_BYTES
    );

    let over = envelope_of_len(MAX_CANONICAL_ENVELOPE_BYTES + 1);
    let expected = EnvelopeLimitError::EnvelopeTooLarge {
        len: MAX_CANONICAL_ENVELOPE_BYTES + 1,
    };
    assert_limit(encode_envelope_v0(&over), expected);
    assert_eq!(check_envelope_v0_limits(&over), Err(expected));
}

// I: escaping counts toward the envelope limit even when the body's UTF-8
// length is far below it.
#[test]
fn i_escaped_body_counts_wire_bytes() {
    let body = "\u{1}".repeat(MAX_CANONICAL_ENVELOPE_BYTES / 6 + 1);
    assert!(body.len() < MAX_CANONICAL_ENVELOPE_BYTES / 5);

    assert!(matches!(
        encode_envelope_v0(&envelope(&body)),
        Err(WireEnvelopeError::Limit(
            EnvelopeLimitError::EnvelopeTooLarge { .. }
        ))
    ));
}

#[test]
fn rendered_errors_do_not_echo_large_values() {
    let base = String::from_utf8(encode_envelope_v0(&envelope("")).unwrap()).unwrap();

    let long_kind = "k".repeat(10_000);
    let wire = base.replace(r#""kind":"message""#, &format!(r#""kind":"{long_kind}""#));
    let rendered = decode_envelope_v0(wire.as_bytes()).unwrap_err().to_string();
    assert!(rendered.len() < 200, "{rendered}");

    let long_id = "i".repeat(10_000);
    let wire = base.replace(r#""id":"msg-limits""#, &format!(r#""id":"{long_id}""#));
    let rendered = decode_envelope_v0(wire.as_bytes()).unwrap_err().to_string();
    assert!(rendered.len() < 300, "{rendered}");

    let long_member = "m".repeat(10_000);
    let wire = base.replace(r#""body":"#, &format!(r#""{long_member}":"#));
    let rendered = decode_envelope_v0(wire.as_bytes()).unwrap_err().to_string();
    assert!(rendered.len() < 400, "{rendered}");
}
