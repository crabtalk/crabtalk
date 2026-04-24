//! Cron scheduler for Crabtalk — binary-side state and runner.
//!
//! Shared types (`CronEntry`, validators) live in `wcore::trigger::cron` so
//! alternative consumers (e.g. multi-tenant schedulers backed by a database)
//! can use them without pulling in the TOML file format or the runner loop.

pub mod runner;
pub mod store;

pub use runner::run;
pub use store::Store;
