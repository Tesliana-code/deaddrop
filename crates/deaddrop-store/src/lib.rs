//! Deaddrop persistence boundaries.
//!
//! Stores preserve durable evidence without becoming semantic or source
//! authority for domains owned elsewhere.

mod delivery_events;
mod messages;
mod sqlite_delivery_events;
mod sqlite_messages;

pub use delivery_events::{
    AppendOutcome, DeliveryEventStore, InMemoryDeliveryEventStore, InMemoryDeliveryEventStoreError,
};
pub use sqlite_delivery_events::{SqliteDeliveryEventStore, SqliteDeliveryEventStoreError};

pub use messages::{
    InMemoryMessageStore, InMemoryMessageStoreError, MessageStore, MessageStoreOutcome,
};
pub use sqlite_messages::{SqliteMessageStore, SqliteMessageStoreError};
