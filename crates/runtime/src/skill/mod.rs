//! Crabtalk skill integration — parsing and tool dispatch.
//!
//! The [`Skill`] domain type and [`SkillRepo`] trait live in core.
//! This module provides the SKILL.md parser (used by daemon's
//! `FsSkillRepo`) and the tool dispatch handler.

pub use wcore::repos::Skill;

pub mod loader;
pub mod tool;
