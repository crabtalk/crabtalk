//! Tool abstractions for the unified LLM Interfaces

use schemars::Schema;

/// A tool for the LLM
pub struct Tool {
    /// The name of the tool
    pub name: &'static str,

    /// The description of the tool
    pub description: &'static str,

    /// The parameters of the tool
    pub parameters: Schema,

    /// Whether to strictly validate the parameters
    pub strict: bool,
}
