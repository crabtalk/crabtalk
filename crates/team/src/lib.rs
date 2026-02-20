//! Pluggable agent teams for Cydonia.
//!
//! This crate re-exports the [`Team`] trait and helpers from `ccore`.
//!
//! Implement [`Team`] on your own struct to compose a leader agent
//! with typed worker agents. No `dyn`, no type erasure.
//!
//! # Example
//!
//! ```rust,ignore
//! use cydonia_team::{Team, tool};
//! use ccore::{Agent, Tool};
//!
//! #[derive(Clone)]
//! struct MyTeam { leader: MyLeader, analyst: Analyst }
//!
//! impl Team for MyTeam {
//!     type Leader = MyLeader;
//!     fn leader(&self) -> &MyLeader { &self.leader }
//!     fn workers(&self) -> Vec<Tool> { vec![tool("analyst", "analysis")] }
//!     async fn call(&self, name: &str, input: String) -> anyhow::Result<String> {
//!         // route to worker by name
//!         todo!()
//!     }
//! }
//! ```

pub use ccore::team::{Team, extract_input, tool};
