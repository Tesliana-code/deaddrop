//! Deaddrop domain semantics and authority boundaries.
//!
//! This crate derives domain meaning from protocol evidence without becoming
//! a transport, persistence, or UI layer.

mod delivery_projection;

pub use delivery_projection::{DeliveryProjection, DeliveryProjectionError};
