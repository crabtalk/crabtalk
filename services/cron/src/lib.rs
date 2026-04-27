//! Cron scheduler for Crabtalk.
//!
//! Desktop-oriented: single-tenant, TOML-backed, fires `/{skill}` into the
//! daemon via the SDK client. Alternate consumers (e.g. multi-tenant cloud
//! schedulers) model their own entry shape and storage — this crate is not
//! a generic scheduling library.

pub mod entry;
pub mod runner;
pub mod store;

pub use entry::{CronEntry, is_quiet, validate_schedule};
pub use runner::run;
pub use store::Store;
