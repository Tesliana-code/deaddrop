use std::collections::HashSet;

use deaddrop_protocol::{
    EnvelopeV0, MessageId, MessageKind, NodeId, decode_envelope_v0, encode_envelope_v0,
};

fn envelope(id: MessageId) -> EnvelopeV0 {
    EnvelopeV0::new(
        id,
        NodeId::parse("node-a:agent:deaddrop").unwrap(),
        NodeId::parse("node-b:agent:deaddrop").unwrap(),
        MessageKind::Message,
        "hello",
    )
}

#[test]
fn a_generated_ids_satisfy_v0_identifier_validation() {
    for _ in 0..100 {
        let id = MessageId::generate();
        assert_eq!(MessageId::parse(id.as_str()), Ok(id.clone()));
    }
}

#[test]
fn b_successive_generated_ids_are_distinct() {
    let ids: HashSet<MessageId> = (0..10_000).map(|_| MessageId::generate()).collect();
    assert_eq!(ids.len(), 10_000);
}

#[test]
fn c_generated_id_round_trips_through_canonical_wire() {
    let original = envelope(MessageId::generate());

    let bytes = encode_envelope_v0(&original).unwrap();
    let decoded = decode_envelope_v0(&bytes).unwrap();

    assert_eq!(decoded, original);
    assert_eq!(encode_envelope_v0(&decoded).unwrap(), bytes);
}

#[test]
fn d_custom_opaque_ids_still_parse_and_round_trip() {
    for value in [
        "msg-wire-1",
        "1",
        "node-a:outbox:42",
        "message with interior spaces",
        "nachricht-ä-✉",
        "01890a5d-ac96-774b-bcce-b302099a8057",
    ] {
        let id = MessageId::parse(value).unwrap_or_else(|error| panic!("{value:?}: {error}"));
        assert_eq!(id.as_str(), value);

        let bytes = encode_envelope_v0(&envelope(id.clone())).unwrap();
        assert_eq!(decode_envelope_v0(&bytes).unwrap().id(), &id);
    }
}
