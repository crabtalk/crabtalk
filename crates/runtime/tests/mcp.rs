//! Tests for the MCP bridge.

use std::sync::Arc;
use walrus_runtime::mcp::{McpBridge, convert_tool};

#[test]
fn tool_conversion() {
    let input_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "description": "URL to fetch" }
        },
        "required": ["url"]
    });
    let json_obj: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(input_schema).unwrap();

    let mcp_tool = rmcp::model::Tool {
        name: "fetch".into(),
        title: None,
        description: Some("Fetch a URL".into()),
        input_schema: Arc::new(json_obj),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    };

    let walrus_tool = convert_tool(&mcp_tool);
    assert_eq!(walrus_tool.name, "fetch");
    assert_eq!(walrus_tool.description, "Fetch a URL");
    assert!(!walrus_tool.strict);

    // Verify the schema was converted.
    let schema_value = serde_json::to_value(&walrus_tool.parameters).unwrap();
    assert_eq!(schema_value["type"], "object");
    assert!(schema_value["properties"]["url"].is_object());
}

#[tokio::test]
async fn mcp_bridge_no_peers() {
    let bridge = McpBridge::new();
    let result = bridge.call("missing_tool", "{}").await;
    assert!(result.contains("not available"));
}

#[tokio::test]
async fn mcp_bridge_empty_tools() {
    let bridge = McpBridge::new();
    let tools = bridge.tools().await;
    assert!(tools.is_empty());
}
