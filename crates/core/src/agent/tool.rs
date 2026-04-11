//! Tool registry, dispatcher trait, and handler types.
//!
//! [`ToolRegistry`] stores `crabllm_core::Tool` schemas by name — no
//! handlers, no closures. [`ToolDispatcher`] is the trait Agents call to
//! execute a tool call; [`ToolHandler`] is the per-tool async closure
//! type stored in a [`ToolEntry`].

use crate::model::HistoryEntry;
use crabllm_core::{FunctionDef, Tool, ToolType};
use heck::ToSnakeCase;
use schemars::JsonSchema;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

/// Boxed future returned by a [`ToolDispatcher::dispatch`] call.
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

/// Dynamic tool dispatch surface.
///
/// The Agent holds an `Arc<dyn ToolDispatcher>` and calls `dispatch` for
/// every tool call the model emits. Implementors look the tool up by
/// name, enforce scope, and invoke the registered handler.
pub trait ToolDispatcher: Send + Sync + 'static {
    fn dispatch<'a>(
        &'a self,
        name: &'a str,
        args: &'a str,
        agent: &'a str,
        sender: &'a str,
        conversation_id: Option<u64>,
    ) -> ToolFuture<'a>;
}

/// Arguments passed to a tool handler during dispatch.
pub struct ToolDispatch {
    /// JSON-encoded arguments string.
    pub args: String,
    /// Name of the agent making this call.
    pub agent: String,
    /// Sender identity (empty for local/owner conversations).
    pub sender: String,
    /// Conversation ID, if running within a conversation.
    pub conversation_id: Option<u64>,
}

/// A type-erased async tool handler.
pub type ToolHandler = Arc<
    dyn Fn(ToolDispatch) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Callback invoked before each agent run to inject context entries.
pub type BeforeRunHook = Arc<dyn Fn(&[HistoryEntry]) -> Vec<HistoryEntry> + Send + Sync>;

/// A registered tool: schema + handler + optional lifecycle hooks.
pub struct ToolEntry {
    /// Tool schema for the LLM.
    pub schema: Tool,
    /// Dispatch handler.
    pub handler: ToolHandler,
    /// Appended to agent system prompt at build time.
    pub system_prompt: Option<String>,
    /// Injected before each agent turn (auto-recall, context, etc).
    pub before_run: Option<BeforeRunHook>,
}

/// Schema-only registry of named tools.
///
/// Stores `crabllm_core::Tool` definitions keyed by function name. Used by
/// `Runtime` to filter tool schemas per agent at `add_agent` time. No
/// handlers or closures are stored here.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: BTreeMap<String, Tool>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a tool schema, keyed by its function name.
    pub fn insert(&mut self, tool: Tool) {
        self.tools.insert(tool.function.name.clone(), tool);
    }

    /// Insert multiple tool schemas.
    pub fn insert_all(&mut self, tools: Vec<Tool>) {
        for tool in tools {
            self.insert(tool);
        }
    }

    /// Remove a tool by name. Returns `true` if it existed.
    pub fn remove(&mut self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    /// Check if a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Return all tool schemas as a `Vec`.
    pub fn tools(&self) -> Vec<Tool> {
        self.tools.values().cloned().collect()
    }

    /// Build a filtered list of tool schemas matching the given names.
    ///
    /// If `names` is empty, all tools are returned. Used by `Runtime::add_agent`
    /// to build the per-agent schema snapshot stored on `Agent`.
    pub fn filtered_snapshot(&self, names: &[String]) -> Vec<Tool> {
        if names.is_empty() {
            return self.tools();
        }
        self.tools
            .iter()
            .filter(|(k, _)| names.iter().any(|n| n == *k))
            .map(|(_, v)| v.clone())
            .collect()
    }
}

/// Trait to provide a description for a tool.
pub trait ToolDescription {
    /// The description of the tool.
    const DESCRIPTION: &'static str;
}

/// Trait to convert a type into a `crabllm_core::Tool`.
pub trait AsTool: ToolDescription {
    /// Convert the type into a `crabllm_core::Tool` (the enveloped
    /// `{kind, function}` wire shape).
    fn as_tool() -> Tool;
}

impl<T> AsTool for T
where
    T: JsonSchema + ToolDescription,
{
    fn as_tool() -> Tool {
        // `strict: None` matches the prior wire behavior: the wcore
        // `Tool.strict: bool` field was set to `true` by every `AsTool` impl
        // but silently dropped by the converter (old convert::to_ct_tool
        // hard-coded `strict: None`). Turning on strict-mode validation
        // here would be a behavior change masquerading as a refactor —
        // leave any opt-in to a separate commit that validates every tool
        // schema.
        Tool {
            kind: ToolType::Function,
            function: FunctionDef {
                name: T::schema_name().to_snake_case(),
                description: Some(Self::DESCRIPTION.into()),
                parameters: Some(
                    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default(),
                ),
            },
            strict: None,
        }
    }
}
