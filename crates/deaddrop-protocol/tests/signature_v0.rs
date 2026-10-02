//! Detached SignatureV0 wire contract.
//!
//! A signature record is data only: this crate freezes its canonical shape
//! and field formats and performs no cryptography. EnvelopeV0 is unchanged.

use deaddrop_protocol::{
    Ed25519PublicKey, MessageId, NodeId, SIGNATURE_PROTOCOL_V0, SignatureV0, WireSignatureError,
    decode_signature_v0, encode_signature_v0,
};

const KEY_HEX: &str = "0101010101010101010101010101010101010101010101010101010101010101";
const SIG_HEX: &str = concat!(
    "0202020202020202020202020202020202020202020202020202020202020202",
    "0202020202020202020202020202020202020202020202020202020202020202"
);

fn canonical() -> String {
    format!(
        concat!(
            r#"{{"protocol":"deaddrop-sig/0","message_id":"msg-1","#,
            r#""signer":"node-a:shell:deaddrop","#,
            r#""key":"ed25519:{key}","#,
            r#""signature":"{sig}"}}"#
        ),
        key = KEY_HEX,
        sig = SIG_HEX
    )
}

fn record() -> SignatureV0 {
    SignatureV0::new(
        MessageId::parse("msg-1").unwrap(),
        NodeId::parse("node-a:shell:deaddrop").unwrap(),
        Ed25519PublicKey::from_bytes([1; 32]),
        [2; 64],
    )
}

#[test]
fn protocol_name_is_distinct_from_envelope_protocol() {
    assert_eq!(SIGNATURE_PROTOCOL_V0, "deaddrop-sig/0");
    assert_ne!(SIGNATURE_PROTOCOL_V0, deaddrop_protocol::PROTOCOL_V0);
}

#[test]
fn encodes_one_canonical_form() {
    assert_eq!(
        String::from_utf8(encode_signature_v0(&record()).unwrap()).unwrap(),
        canonical()
    );
}

#[test]
fn decodes_canonical_bytes_exactly() {
    let decoded = decode_signature_v0(canonical().as_bytes()).unwrap();
    assert_eq!(decoded, record());
    assert_eq!(decoded.message_id().as_str(), "msg-1");
    assert_eq!(decoded.signer().as_str(), "node-a:shell:deaddrop");
    assert_eq!(decoded.key().as_bytes(), &[1; 32]);
    assert_eq!(decoded.signature(), &[2; 64]);
}

#[test]
fn public_key_text_form_round_trips() {
    let text = format!("ed25519:{KEY_HEX}");
    let key: Ed25519PublicKey = text.parse().unwrap();
    assert_eq!(key.to_string(), text);
}

#[test]
fn public_key_rejects_other_schemes_lengths_and_uppercase() {
    for bad in [
        format!("rsa:{KEY_HEX}"),
        KEY_HEX.to_owned(),
        format!("ed25519:{}", &KEY_HEX[2..]),
        format!("ed25519:{}", "AB".repeat(32)),
        format!("ed25519:{}zz", &KEY_HEX[2..]),
    ] {
        assert!(bad.parse::<Ed25519PublicKey>().is_err(), "{bad}");
    }
}

#[test]
fn rejects_unknown_fields() {
    let extended = canonical().replace(r#""signature""#, r#""extra":1,"signature""#);
    assert!(matches!(
        decode_signature_v0(extended.as_bytes()),
        Err(WireSignatureError::Json(_))
    ));
}

#[test]
fn rejects_other_protocols() {
    let other = canonical().replace("deaddrop-sig/0", "deaddrop-sig/1");
    assert!(matches!(
        decode_signature_v0(other.as_bytes()),
        Err(WireSignatureError::UnsupportedProtocol { .. })
    ));
}

#[test]
fn rejects_non_canonical_encoding() {
    let spaced = canonical().replace(r#"","signer""#, r#"", "signer""#);
    assert!(matches!(
        decode_signature_v0(spaced.as_bytes()),
        Err(WireSignatureError::NonCanonicalEncoding)
    ));
}

#[test]
fn rejects_malformed_key_and_signature_fields() {
    let short_sig = canonical().replace(SIG_HEX, &SIG_HEX[2..]);
    assert!(matches!(
        decode_signature_v0(short_sig.as_bytes()),
        Err(WireSignatureError::InvalidField {
            field: "signature",
            ..
        })
    ));
    let bad_key = canonical().replace("ed25519:", "x25519:");
    assert!(matches!(
        decode_signature_v0(bad_key.as_bytes()),
        Err(WireSignatureError::InvalidField { field: "key", .. })
    ));
    let empty_id = canonical().replace(r#""msg-1""#, r#""""#);
    assert!(matches!(
        decode_signature_v0(empty_id.as_bytes()),
        Err(WireSignatureError::InvalidField {
            field: "message_id",
            ..
        })
    ));
}
