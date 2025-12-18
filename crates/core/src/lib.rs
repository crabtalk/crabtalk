//! Core abstractions for Unified LLM Interface

pub use {
    config::Config,
    message::{Message, Role},
    provider::LLM,
    tool::Tool,
};

mod config;
mod message;
mod provider;
mod tool;
