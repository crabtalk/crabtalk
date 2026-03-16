//! Walrus Extension protocol types.
//!
//! Generated from `proto/ext.proto`. Re-exported here for stable
//! `wcore::protocol::ext::*` import paths.

pub use crate::protocol::proto::ext_proto::{
    AfterRunCap, BeforeRunCap, BuildAgentCap, Capability, CompactCap, EventObserverCap,
    ExtAfterRun, ExtAfterRunResult, ExtBeforeRun, ExtBeforeRunResult, ExtBuildAgent,
    ExtBuildAgentResult, ExtCompact, ExtCompactResult, ExtConfigure, ExtConfigured, ExtError,
    ExtEvent, ExtGetSchema, ExtHello, ExtInferRequest, ExtInferResult, ExtReady, ExtRegisterTools,
    ExtRequest, ExtResponse, ExtSchemaResult, ExtServiceQuery, ExtServiceQueryResult, ExtShutdown,
    ExtToolCall, ExtToolResult, ExtToolSchemas, InferCap, QueryCap, SimpleMessage, ToolDef,
    ToolsList, capability, ext_request, ext_response,
};
