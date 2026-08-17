//! Persistence backends for Crabtalk.
//!
//! sqlite is what this repository ships. The cloud runs postgres against the
//! same [`Storage`](crate::Storage) trait — which is why it is a trait.

mod sqlite;

pub use sqlite::SqliteStorage;
