//! Walrus skill registry — tag-indexed skill matching and prompt enrichment.
//!
//! Skills are named units of agent behavior loaded from Markdown files with
//! YAML frontmatter (agentskills.io format). The [`SkillRegistry`] indexes
//! skills by tags for dynamic discovery via `search_skill` and `load_skill`.

pub use {
    handler::SkillHandler,
    registry::{Skill, SkillRegistry},
};

mod handler;
pub mod loader;
pub mod registry;
