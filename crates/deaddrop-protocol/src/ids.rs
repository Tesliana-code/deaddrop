use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdError {
    Empty,
    LeadingOrTrailingWhitespace,
    ControlCharacter,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "identifier must not be empty"),
            Self::LeadingOrTrailingWhitespace => {
                write!(
                    f,
                    "identifier must not contain leading or trailing whitespace"
                )
            }
            Self::ControlCharacter => {
                write!(f, "identifier must not contain control characters")
            }
        }
    }
}

impl std::error::Error for IdError {}

fn validate_id(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }

    if value.trim() != value {
        return Err(IdError::LeadingOrTrailingWhitespace);
    }

    if value.chars().any(char::is_control) {
        return Err(IdError::ControlCharacter);
    }

    Ok(())
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_id(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }
    };
}

opaque_id!(NodeId);
opaque_id!(MessageId);
opaque_id!(DeliveryEventId);
opaque_id!(CorrelationId);

impl MessageId {
    /// Reference sender-side generator for a new logical message.
    ///
    /// Produces a random (version 4) UUID in lowercase hyphenated form from
    /// the operating system CSPRNG: globally collision resistant without
    /// coordination, clocks, counters, or relay/recipient assignment, and
    /// carrying no timestamp or ordering metadata.
    ///
    /// This is a reference format, not a wire restriction: `parse` continues
    /// to accept every opaque identifier valid under V0. Retransmissions of
    /// one logical message reuse its id; they never generate a new one.
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().hyphenated().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_opaque_node_identity_without_freezing_shape() {
        let id = NodeId::parse("host:harness:project").expect("valid node id");
        assert_eq!(id.as_str(), "host:harness:project");
    }

    #[test]
    fn rejects_empty_identifier() {
        assert_eq!(MessageId::parse(""), Err(IdError::Empty));
    }

    #[test]
    fn rejects_leading_or_trailing_whitespace() {
        assert_eq!(
            CorrelationId::parse(" corr-1"),
            Err(IdError::LeadingOrTrailingWhitespace)
        );
    }

    #[test]
    fn rejects_control_characters() {
        assert_eq!(MessageId::parse("msg\n1"), Err(IdError::ControlCharacter));
    }
}
