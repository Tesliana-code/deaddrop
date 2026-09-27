use std::fmt;
use std::path::Path;

use deaddrop_protocol::{DeliveryEvent, DeliveryEventId, DeliveryEventKind, MessageId, NodeId};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::{AppendOutcome, DeliveryEventStore};

#[derive(Debug)]
pub enum SqliteDeliveryEventStoreError {
    Sqlite(rusqlite::Error),
    EventIdentityConflict { event_id: DeliveryEventId },
    InvalidPersistedField { field: &'static str, value: String },
    UnknownPersistedKind { value: String },
}

impl fmt::Display for SqliteDeliveryEventStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "sqlite error: {error}"),
            Self::EventIdentityConflict { event_id } => write!(
                f,
                "delivery event id {event_id} already identifies different evidence"
            ),
            Self::InvalidPersistedField { field, value } => {
                write!(f, "invalid persisted {field}: {value:?}")
            }
            Self::UnknownPersistedKind { value } => {
                write!(f, "unknown persisted delivery event kind: {value:?}")
            }
        }
    }
}

impl std::error::Error for SqliteDeliveryEventStoreError {}

impl From<rusqlite::Error> for SqliteDeliveryEventStoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub struct SqliteDeliveryEventStore {
    connection: Connection,
}

impl SqliteDeliveryEventStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteDeliveryEventStoreError> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> Result<Self, SqliteDeliveryEventStoreError> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> Result<Self, SqliteDeliveryEventStoreError> {
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS delivery_events (
                local_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL UNIQUE,
                message_id TEXT NOT NULL,
                reported_by TEXT NOT NULL,
                kind TEXT NOT NULL CHECK (
                    kind IN (
                        'transport_accepted',
                        'recipient_received',
                        'recipient_verified',
                        'recipient_acknowledged',
                        'delivery_failed',
                        'delivery_expired'
                    )
                )
            );

            CREATE INDEX IF NOT EXISTS
                delivery_events_message_sequence_idx
            ON delivery_events(message_id, local_sequence);
            ",
        )?;

        Ok(Self { connection })
    }
}

impl DeliveryEventStore for SqliteDeliveryEventStore {
    type Error = SqliteDeliveryEventStoreError;

    fn append(&mut self, event: DeliveryEvent) -> Result<AppendOutcome, Self::Error> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing = transaction
            .query_row(
                "
                SELECT message_id, reported_by, kind
                FROM delivery_events
                WHERE event_id = ?1
                ",
                params![event.id().as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;

        if let Some((message_id, reported_by, kind)) = existing {
            if message_id == event.message_id().as_str()
                && reported_by == event.reported_by().as_str()
                && kind == event.kind().as_str()
            {
                return Ok(AppendOutcome::AlreadyPresent);
            }

            return Err(SqliteDeliveryEventStoreError::EventIdentityConflict {
                event_id: event.id().clone(),
            });
        }

        transaction.execute(
            "
            INSERT INTO delivery_events (
                event_id,
                message_id,
                reported_by,
                kind
            )
            VALUES (?1, ?2, ?3, ?4)
            ",
            params![
                event.id().as_str(),
                event.message_id().as_str(),
                event.reported_by().as_str(),
                event.kind().as_str(),
            ],
        )?;

        transaction.commit()?;

        Ok(AppendOutcome::Appended)
    }

    fn load_for_message(&self, message_id: &MessageId) -> Result<Vec<DeliveryEvent>, Self::Error> {
        let mut statement = self.connection.prepare(
            "
            SELECT event_id, message_id, reported_by, kind
            FROM delivery_events
            WHERE message_id = ?1
            ORDER BY local_sequence ASC
            ",
        )?;

        let rows = statement.query_map(params![message_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut events = Vec::new();

        for row in rows {
            let (event_id, stored_message_id, reported_by, kind) = row?;

            let event_id = DeliveryEventId::parse(event_id.clone()).map_err(|_| {
                SqliteDeliveryEventStoreError::InvalidPersistedField {
                    field: "event_id",
                    value: event_id,
                }
            })?;

            let stored_message_id = MessageId::parse(stored_message_id.clone()).map_err(|_| {
                SqliteDeliveryEventStoreError::InvalidPersistedField {
                    field: "message_id",
                    value: stored_message_id,
                }
            })?;

            let reported_by = NodeId::parse(reported_by.clone()).map_err(|_| {
                SqliteDeliveryEventStoreError::InvalidPersistedField {
                    field: "reported_by",
                    value: reported_by,
                }
            })?;

            let kind = DeliveryEventKind::parse(&kind).ok_or_else(|| {
                SqliteDeliveryEventStoreError::UnknownPersistedKind { value: kind }
            })?;

            events.push(DeliveryEvent::new(
                event_id,
                stored_message_id,
                reported_by,
                kind,
            ));
        }

        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, message: &str, reporter: &str, kind: DeliveryEventKind) -> DeliveryEvent {
        DeliveryEvent::new(
            DeliveryEventId::parse(id).unwrap(),
            MessageId::parse(message).unwrap(),
            NodeId::parse(reporter).unwrap(),
            kind,
        )
    }

    #[test]
    fn exact_replay_is_idempotent() {
        let mut store = SqliteDeliveryEventStore::open_in_memory().unwrap();

        let evidence = event("e1", "msg-1", "relay-a", DeliveryEventKind::DeliveryFailed);

        assert_eq!(
            store.append(evidence.clone()).unwrap(),
            AppendOutcome::Appended
        );

        assert_eq!(
            store.append(evidence).unwrap(),
            AppendOutcome::AlreadyPresent
        );
    }

    #[test]
    fn conflicting_event_identity_fails_closed() {
        let mut store = SqliteDeliveryEventStore::open_in_memory().unwrap();

        store
            .append(event(
                "collision-1",
                "msg-1",
                "relay-a",
                DeliveryEventKind::DeliveryFailed,
            ))
            .unwrap();

        let result = store.append(event(
            "collision-1",
            "msg-1",
            "node-b",
            DeliveryEventKind::RecipientReceived,
        ));

        assert!(matches!(
            result,
            Err(SqliteDeliveryEventStoreError::EventIdentityConflict { .. })
        ));
    }

    #[test]
    fn message_scoped_reads_preserve_local_append_order() {
        let mut store = SqliteDeliveryEventStore::open_in_memory().unwrap();

        store
            .append(event(
                "e1",
                "msg-a",
                "relay-a",
                DeliveryEventKind::TransportAccepted,
            ))
            .unwrap();

        store
            .append(event(
                "other",
                "msg-b",
                "relay-z",
                DeliveryEventKind::DeliveryFailed,
            ))
            .unwrap();

        store
            .append(event(
                "e2",
                "msg-a",
                "node-b",
                DeliveryEventKind::RecipientReceived,
            ))
            .unwrap();

        let loaded = store
            .load_for_message(&MessageId::parse("msg-a").unwrap())
            .unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id().as_str(), "e1");
        assert_eq!(loaded[1].id().as_str(), "e2");
    }
}
