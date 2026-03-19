//! Crabtalk skill registry — skill matching and prompt enrichment.
//!
//! Skills are named units of agent behavior loaded from Markdown files with
//! YAML frontmatter (agentskills.io format). The [`SkillRegistry`] indexes
//! skills for discovery via the `skill` tool.

pub use {
    handler::SkillHandler,
    registry::{Skill, SkillRegistry},
};

mod handler;
pub mod loader;
pub mod registry;
pub(crate) mod tool;
