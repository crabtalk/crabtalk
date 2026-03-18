//! Walrus Extension protocol types.
//!
//! Generated from `proto/ext.proto`. Re-exported here for stable
//! `wcore::protocol::ext::*` import paths.

pub use crate::protocol::proto::ext_proto::{
    Capability, ExtConfigure, ExtConfigured, ExtError, ExtGetSchema, ExtHello, ExtReady,
    ExtRegisterTools, ExtRequest, ExtResponse, ExtSchemaResult, ExtServiceQuery,
    ExtServiceQueryResult, ExtShutdown, ExtToolCall, ExtToolResult, ExtToolSchemas, QueryCap,
    ToolDef, ToolsList, capability, ext_request, ext_response,
};
