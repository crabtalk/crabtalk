//! Crabtalk skill registry — skill matching and prompt enrichment.

pub use {
    handler::SkillHandler,
    registry::{Skill, SkillRegistry},
};

mod handler;
pub mod loader;
pub mod registry;
pub mod tool;
