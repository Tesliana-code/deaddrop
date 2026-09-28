use std::fmt;
use std::path::Path;
use std::str::FromStr;

use deaddrop_protocol::{
    ArtifactRef, CorrelationId, EnvelopeV0, MessageId, MessageKind, NodeId, PROTOCOL_V0,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::{MessageStore, MessageStoreOutcome};

#[derive(Debug)]
pub enum SqliteMessageStoreError {
    Sqlite(rusqlite::Error),
    MessageIdentityConflict { message_id: MessageId },
    InvalidPersistedField { field: &'static str, value: String },
    UnknownPersistedKind { value: String },
    UnsupportedProtocol { value: String },
}

impl fmt::Display for SqliteMessageStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "sqlite error: {error}"),
            Self::MessageIdentityConflict { message_id } => write!(
                f,
                "message id {message_id} already identifies a different envelope"
            ),
            Self::InvalidPersistedField { field, value } => {
                write!(f, "invalid persisted {field}: {value:?}")
            }
            Self::UnknownPersistedKind { value } => {
                write!(f, "unknown persisted message kind: {value:?}")
            }
            Self::UnsupportedProtocol { value } => {
                write!(f, "unsupported persisted protocol: {value:?}")
            }
        }
    }
}

impl std::error::Error for SqliteMessageStoreError {}

impl From<rusqlite::Error> for SqliteMessageStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub struct SqliteMessageStore {
    connection: Connection,
}

impl SqliteMessageStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteMessageStoreError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, SqliteMessageStoreError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, SqliteMessageStoreError> {
        connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS messages (
                message_id TEXT PRIMARY KEY,
                protocol TEXT NOT NULL,
                sender TEXT NOT NULL,
                recipient TEXT NOT NULL,
                kind TEXT NOT NULL,
                correlation_id TEXT NULL,
                body TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS message_artifact_refs (
                message_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                artifact_ref TEXT NOT NULL,
                PRIMARY KEY (message_id, ordinal),
                FOREIGN KEY (message_id)
                    REFERENCES messages(message_id)
                    ON DELETE CASCADE
            );
            ",
        )?;

        Ok(Self { connection })
    }

    fn load_from_transaction(
        transaction: &Transaction<'_>,
        message_id: &MessageId,
    ) -> Result<Option<EnvelopeV0>, SqliteMessageStoreError> {
        let row = transaction
            .query_row(
                "
                SELECT protocol, sender, recipient, kind, correlation_id, body
                FROM messages
                WHERE message_id = ?1
                ",
                params![message_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?;

        let Some((protocol, sender, recipient, kind, correlation_id, body)) = row else {
            return Ok(None);
        };

        let envelope = Self::rebuild_envelope(
            transaction,
            message_id,
            protocol,
            sender,
            recipient,
            kind,
            correlation_id,
            body,
        )?;

        Ok(Some(envelope))
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_envelope(
        transaction: &Transaction<'_>,
        message_id: &MessageId,
        protocol: String,
        sender: String,
        recipient: String,
        kind: String,
        correlation_id: Option<String>,
        body: String,
    ) -> Result<EnvelopeV0, SqliteMessageStoreError> {
        if protocol != PROTOCOL_V0 {
            return Err(SqliteMessageStoreError::UnsupportedProtocol { value: protocol });
        }

        let sender = NodeId::parse(sender.clone()).map_err(|_| {
            SqliteMessageStoreError::InvalidPersistedField {
                field: "sender",
                value: sender,
            }
        })?;

        let recipient = NodeId::parse(recipient.clone()).map_err(|_| {
            SqliteMessageStoreError::InvalidPersistedField {
                field: "recipient",
                value: recipient,
            }
        })?;

        let kind = MessageKind::parse(&kind)
            .ok_or_else(|| SqliteMessageStoreError::UnknownPersistedKind { value: kind })?;

        let correlation_id = correlation_id
            .map(|value| {
                CorrelationId::parse(value.clone()).map_err(|_| {
                    SqliteMessageStoreError::InvalidPersistedField {
                        field: "correlation_id",
                        value,
                    }
                })
            })
            .transpose()?;

        let mut envelope = EnvelopeV0::new(message_id.clone(), sender, recipient, kind, body);

        if let Some(correlation_id) = correlation_id {
            envelope = envelope.with_correlation_id(correlation_id);
        }

        let mut statement = transaction.prepare(
            "
            SELECT artifact_ref
            FROM message_artifact_refs
            WHERE message_id = ?1
            ORDER BY ordinal ASC
            ",
        )?;

        let refs =
            statement.query_map(params![message_id.as_str()], |row| row.get::<_, String>(0))?;

        for artifact_ref in refs {
            let artifact_ref = artifact_ref?;

            let parsed = ArtifactRef::from_str(&artifact_ref).map_err(|_| {
                SqliteMessageStoreError::InvalidPersistedField {
                    field: "artifact_ref",
                    value: artifact_ref,
                }
            })?;

            envelope = envelope.with_artifact_ref(parsed);
        }

        Ok(envelope)
    }
}

impl MessageStore for SqliteMessageStore {
    type Error = SqliteMessageStoreError;

    fn store(&mut self, envelope: EnvelopeV0) -> Result<MessageStoreOutcome, Self::Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        if let Some(existing) = Self::load_from_transaction(&transaction, envelope.id())? {
            if existing == envelope {
                return Ok(MessageStoreOutcome::AlreadyPresent);
            }

            return Err(SqliteMessageStoreError::MessageIdentityConflict {
                message_id: envelope.id().clone(),
            });
        }

        transaction.execute(
            "
            INSERT INTO messages (
                message_id,
                protocol,
                sender,
                recipient,
                kind,
                correlation_id,
                body
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                envelope.id().as_str(),
                envelope.protocol(),
                envelope.from().as_str(),
                envelope.to().as_str(),
                envelope.kind().as_str(),
                envelope.correlation_id().map(|value| value.as_str()),
                envelope.body(),
            ],
        )?;

        for (ordinal, artifact_ref) in envelope.artifact_refs().iter().enumerate() {
            transaction.execute(
                "
                INSERT INTO message_artifact_refs (
                    message_id,
                    ordinal,
                    artifact_ref
                )
                VALUES (?1, ?2, ?3)
                ",
                params![
                    envelope.id().as_str(),
                    ordinal as i64,
                    artifact_ref.to_string(),
                ],
            )?;
        }

        transaction.commit()?;

        Ok(MessageStoreOutcome::Stored)
    }

    fn load(&self, message_id: &MessageId) -> Result<Option<EnvelopeV0>, Self::Error> {
        let transaction = self.connection.unchecked_transaction()?;

        let envelope = Self::load_from_transaction(&transaction, message_id)?;

        transaction.commit()?;

        Ok(envelope)
    }
}
