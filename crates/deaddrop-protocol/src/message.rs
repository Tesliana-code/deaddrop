use crate::{ArtifactRef, CorrelationId, MessageId, NodeId};

pub const PROTOCOL_V0: &str = "deaddrop/0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKind {
    Message,
    Request,
    Response,
    Handoff,
    TaskClaim,
    Checkpoint,
    Acknowledgment,
    Error,
    CapabilityDeclaration,
}

impl MessageKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Request => "request",
            Self::Response => "response",
            Self::Handoff => "handoff",
            Self::TaskClaim => "task_claim",
            Self::Checkpoint => "checkpoint",
            Self::Acknowledgment => "acknowledgment",
            Self::Error => "error",
            Self::CapabilityDeclaration => "capability_declaration",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "message" => Some(Self::Message),
            "request" => Some(Self::Request),
            "response" => Some(Self::Response),
            "handoff" => Some(Self::Handoff),
            "task_claim" => Some(Self::TaskClaim),
            "checkpoint" => Some(Self::Checkpoint),
            "acknowledgment" => Some(Self::Acknowledgment),
            "error" => Some(Self::Error),
            "capability_declaration" => Some(Self::CapabilityDeclaration),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeV0 {
    id: MessageId,
    from: NodeId,
    to: NodeId,
    kind: MessageKind,
    correlation_id: Option<CorrelationId>,
    body: String,
    artifact_refs: Vec<ArtifactRef>,
}

impl EnvelopeV0 {
    pub fn new(
        id: MessageId,
        from: NodeId,
        to: NodeId,
        kind: MessageKind,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id,
            from,
            to,
            kind,
            correlation_id: None,
            body: body.into(),
            artifact_refs: Vec::new(),
        }
    }

    pub fn protocol(&self) -> &'static str {
        PROTOCOL_V0
    }

    pub fn id(&self) -> &MessageId {
        &self.id
    }

    pub fn from(&self) -> &NodeId {
        &self.from
    }

    pub fn to(&self) -> &NodeId {
        &self.to
    }

    pub fn kind(&self) -> MessageKind {
        self.kind
    }

    pub fn correlation_id(&self) -> Option<&CorrelationId> {
        self.correlation_id.as_ref()
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn artifact_refs(&self) -> &[ArtifactRef] {
        &self.artifact_refs
    }

    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    pub fn with_artifact_ref(mut self, artifact_ref: ArtifactRef) -> Self {
        self.artifact_refs.push(artifact_ref);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn node(value: &str) -> NodeId {
        NodeId::parse(value).expect("valid node id")
    }

    fn message_id(value: &str) -> MessageId {
        MessageId::parse(value).expect("valid message id")
    }

    #[test]
    fn creates_recipient_scoped_v0_message() {
        let envelope = EnvelopeV0::new(
            message_id("msg-1"),
            node("node-a:agent:project"),
            node("node-b:agent:project"),
            MessageKind::Message,
            "hello",
        );

        assert_eq!(envelope.protocol(), "deaddrop/0");
        assert_eq!(envelope.from().as_str(), "node-a:agent:project");
        assert_eq!(envelope.to().as_str(), "node-b:agent:project");
        assert_eq!(envelope.kind(), MessageKind::Message);
        assert_eq!(envelope.body(), "hello");
        assert_eq!(envelope.correlation_id(), None);
        assert!(envelope.artifact_refs().is_empty());
    }

    #[test]
    fn correlation_is_explicit_and_optional() {
        let envelope = EnvelopeV0::new(
            message_id("msg-2"),
            node("node-a"),
            node("node-b"),
            MessageKind::Handoff,
            "continue this task",
        )
        .with_correlation_id(CorrelationId::parse("corr-7").expect("valid correlation id"));

        assert_eq!(
            envelope
                .correlation_id()
                .expect("correlation exists")
                .as_str(),
            "corr-7"
        );
    }

    #[test]
    fn carries_immutable_artifact_references() {
        let artifact = ArtifactRef::from_str(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("valid artifact ref");

        let envelope = EnvelopeV0::new(
            message_id("msg-3"),
            node("node-a"),
            node("node-b"),
            MessageKind::Handoff,
            "artifact attached by reference",
        )
        .with_artifact_ref(artifact.clone());

        assert_eq!(envelope.artifact_refs(), &[artifact]);
    }

    #[test]
    fn message_kind_has_stable_protocol_name() {
        assert_eq!(MessageKind::TaskClaim.as_str(), "task_claim");
        assert_eq!(
            MessageKind::CapabilityDeclaration.as_str(),
            "capability_declaration"
        );
    }
}
