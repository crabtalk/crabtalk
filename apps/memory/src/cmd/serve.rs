//! WHS serve command — run wmemory as a hook service over UDS.

use crate::{
    config::MemoryConfig,
    dispatch::{MemoryService, truncate_utf8},
    lance::Direction,
    tool,
};
use std::path::Path;
use wcore::{
    agent::AsTool,
    model::Tool,
    protocol::{
        PROTOCOL_VERSION,
        codec::{read_message, write_message},
        whs::{
            BeforeRunCap, BuildAgentCap, Capability, CompactCap, EventObserverCap, QueryCap,
            SimpleMessage, ToolDef, ToolsList, WhsBeforeRunResult, WhsBuildAgentResult,
            WhsCompactResult, WhsConfigured, WhsError, WhsReady, WhsRequest, WhsResponse,
            WhsServiceQueryResult, WhsToolResult, WhsToolSchemas, capability, whs_request,
            whs_response,
        },
    },
};

pub async fn run(socket: &Path) -> anyhow::Result<()> {
    // Clean up stale socket from a previous run.
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    let listener = tokio::net::UnixListener::bind(socket)?;
    tracing::info!("memory service listening on {}", socket.display());

    let (stream, _) = listener.accept().await?;
    let (mut reader, mut writer) = stream.into_split();

    // ── Hello → Ready ────────────────────────────────────────────────
    let hello: WhsRequest = read_message(&mut reader).await?;
    match hello.msg {
        Some(whs_request::Msg::Hello(_)) => {}
        other => anyhow::bail!("expected Hello, got {other:?}"),
    }

    let tool_names = vec![
        "remember".to_owned(),
        "recall".to_owned(),
        "relate".to_owned(),
        "connections".to_owned(),
        "compact".to_owned(),
        "distill".to_owned(),
        "__journal__".to_owned(),
    ];

    let ready = WhsResponse {
        msg: Some(whs_response::Msg::Ready(WhsReady {
            version: PROTOCOL_VERSION.to_owned(),
            service: "memory".to_owned(),
            capabilities: vec![
                Capability {
                    cap: Some(capability::Cap::Tools(ToolsList { names: tool_names })),
                },
                Capability {
                    cap: Some(capability::Cap::BuildAgent(BuildAgentCap {})),
                },
                Capability {
                    cap: Some(capability::Cap::BeforeRun(BeforeRunCap {})),
                },
                Capability {
                    cap: Some(capability::Cap::Compact(CompactCap {})),
                },
                Capability {
                    cap: Some(capability::Cap::Query(QueryCap {})),
                },
                Capability {
                    cap: Some(capability::Cap::EventObserver(EventObserverCap {})),
                },
            ],
        })),
    };
    write_message(&mut writer, &ready).await?;

    // ── Configure → Configured ───────────────────────────────────────
    let configure: WhsRequest = read_message(&mut reader).await?;
    let config = match configure.msg {
        Some(whs_request::Msg::Configure(c)) => {
            if c.config.is_empty() {
                MemoryConfig::default()
            } else {
                serde_json::from_str(&c.config).unwrap_or_else(|e| {
                    tracing::warn!("invalid config, using defaults: {e}");
                    MemoryConfig::default()
                })
            }
        }
        other => anyhow::bail!("expected Configure, got {other:?}"),
    };
    let configured = WhsResponse {
        msg: Some(whs_response::Msg::Configured(WhsConfigured {})),
    };
    write_message(&mut writer, &configured).await?;

    // ── RegisterTools → ToolSchemas ──────────────────────────────────
    let register: WhsRequest = read_message(&mut reader).await?;
    match register.msg {
        Some(whs_request::Msg::RegisterTools(_)) => {}
        other => anyhow::bail!("expected RegisterTools, got {other:?}"),
    }

    // Build the memory service before constructing dynamic tool schemas.
    let memory_dir = wcore::paths::CONFIG_DIR.join("memory");
    let svc = MemoryService::open(&memory_dir, &config).await?;

    // Build tool defs with dynamic descriptions for remember and relate.
    let tools = build_tool_defs(&svc);
    let schemas = WhsResponse {
        msg: Some(whs_response::Msg::ToolSchemas(WhsToolSchemas { tools })),
    };
    write_message(&mut writer, &schemas).await?;
    tracing::info!("handshake complete");

    // ── Dispatch loop ────────────────────────────────────────────────
    let mut clean_exit = false;
    loop {
        let req: WhsRequest = match read_message(&mut reader).await {
            Ok(r) => r,
            Err(wcore::protocol::codec::FrameError::ConnectionClosed) => {
                tracing::warn!("daemon connection closed");
                break;
            }
            Err(e) => {
                tracing::error!("read error: {e}");
                break;
            }
        };

        let resp = match req.msg {
            Some(whs_request::Msg::ToolCall(call)) => {
                let result = dispatch_tool(&svc, &call.name, &call.args, &call.agent).await;
                WhsResponse {
                    msg: Some(whs_response::Msg::ToolResult(WhsToolResult { result })),
                }
            }
            Some(whs_request::Msg::BuildAgent(ba)) => {
                let result =
                    handle_build_agent(&svc, &ba.name, &ba.description, &ba.system_prompt).await;
                WhsResponse {
                    msg: Some(whs_response::Msg::BuildAgentResult(result)),
                }
            }
            Some(whs_request::Msg::BeforeRun(br)) => {
                let result = handle_before_run(&svc, &br.agent, &br.history).await;
                WhsResponse {
                    msg: Some(whs_response::Msg::BeforeRunResult(result)),
                }
            }
            Some(whs_request::Msg::Compact(_c)) => WhsResponse {
                msg: Some(whs_response::Msg::CompactResult(WhsCompactResult {
                    addition: String::new(),
                })),
            },
            Some(whs_request::Msg::ServiceQuery(sq)) => {
                let result = handle_service_query(&svc, &sq.query).await;
                WhsResponse {
                    msg: Some(whs_response::Msg::ServiceQueryResult(
                        WhsServiceQueryResult { result },
                    )),
                }
            }
            Some(whs_request::Msg::Event(_)) => {
                // Fire-and-forget — no response expected.
                continue;
            }
            Some(whs_request::Msg::GetSchema(_)) => WhsResponse {
                msg: Some(whs_response::Msg::Error(WhsError {
                    message: "schema not yet implemented".into(),
                })),
            },
            Some(whs_request::Msg::Shutdown(_)) => {
                tracing::info!("shutdown requested");
                clean_exit = true;
                break;
            }
            other => WhsResponse {
                msg: Some(whs_response::Msg::Error(WhsError {
                    message: format!("unexpected request: {other:?}"),
                })),
            },
        };

        if let Err(e) = write_message(&mut writer, &resp).await {
            tracing::error!("write error: {e}");
            break;
        }
    }

    // Clean up socket file.
    let _ = std::fs::remove_file(socket);
    if clean_exit {
        Ok(())
    } else {
        anyhow::bail!("connection lost")
    }
}

/// Build tool defs with dynamic descriptions for remember and relate.
fn build_tool_defs(svc: &MemoryService) -> Vec<ToolDef> {
    let remember = Tool {
        description: format!(
            "Store a memory entity. Types: {}.",
            svc.allowed_entities.join(", ")
        )
        .into(),
        ..tool::Remember::as_tool()
    };
    let relate = Tool {
        description: format!(
            "Create a directed relation between two entities by key. Relations: {}.",
            svc.allowed_relations.join(", ")
        )
        .into(),
        ..tool::Relate::as_tool()
    };
    let static_tools = [
        tool::Recall::as_tool(),
        tool::Connections::as_tool(),
        tool::Compact::as_tool(),
        tool::Distill::as_tool(),
    ];

    let mut defs = Vec::with_capacity(6);
    for t in [remember, relate].into_iter().chain(static_tools) {
        defs.push(ToolDef {
            name: t.name.to_string(),
            description: t.description.to_string(),
            parameters: serde_json::to_vec(&t.parameters).expect("schema serialization"),
            strict: t.strict,
        });
    }
    defs
}

/// Dispatch a tool call to the appropriate MemoryService method.
async fn dispatch_tool(svc: &MemoryService, name: &str, args: &str, agent: &str) -> String {
    match name {
        "remember" => svc.dispatch_remember(args).await,
        "recall" => svc.dispatch_recall(args).await,
        "relate" => svc.dispatch_relate(args).await,
        "connections" => svc.dispatch_connections(args).await,
        "compact" => svc.dispatch_compact(agent).await,
        "distill" => svc.dispatch_distill(args, agent).await,
        "__journal__" => svc.dispatch_journal(args, agent).await,
        _ => format!("unknown tool: {name}"),
    }
}

/// Handle the BuildAgent lifecycle event.
///
/// Builds prompt additions: `<self>`, `<identity>`, `<profile>`, `<journal>`
/// blocks plus the memory prompt. Returns tools for registration.
async fn handle_build_agent(
    svc: &MemoryService,
    name: &str,
    description: &str,
    _system_prompt: &str,
) -> WhsBuildAgentResult {
    let lance = &svc.lance;

    // Inject <self> block.
    let mut buf = String::from("\n\n<self>\n");
    buf.push_str(&format!("name: {name}\n"));
    if !description.is_empty() {
        buf.push_str(&format!("description: {description}\n"));
    }
    buf.push_str("</self>");

    // Inject identity entities (shared across all agents).
    if let Ok(identities) = lance.query_by_type("identity", 50).await
        && !identities.is_empty()
    {
        buf.push_str("\n\n<identity>\n");
        for e in &identities {
            buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
        }
        buf.push_str("</identity>");
    }

    // Inject profile entities (shared across all agents).
    if let Ok(profiles) = lance.query_by_type("profile", 50).await
        && !profiles.is_empty()
    {
        buf.push_str("\n\n<profile>\n");
        for e in &profiles {
            buf.push_str(&format!("- **{}**: {}\n", e.key, e.value));
        }
        buf.push_str("</profile>");
    }

    // Inject recent journal entries (agent-scoped).
    if let Ok(journals) = lance.recent_journals(name, 3).await
        && !journals.is_empty()
    {
        buf.push_str("\n\n<journal>\n");
        for j in &journals {
            let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| j.created_at.to_string());
            let summary = truncate_utf8(&j.summary, 500);
            buf.push_str(&format!("- **{ts}**: {summary}\n"));
        }
        buf.push_str("</journal>");
    }

    // Append memory prompt.
    buf.push_str(&format!("\n\n{}", MemoryService::memory_prompt()));

    WhsBuildAgentResult {
        prompt_addition: buf,
        tools: build_tool_defs(svc),
    }
}

/// Handle the BeforeRun lifecycle event.
///
/// Auto-recalls relevant entities, connections, and journal entries based on
/// the last user message via semantic search.
async fn handle_before_run(
    svc: &MemoryService,
    agent: &str,
    history: &[SimpleMessage],
) -> WhsBeforeRunResult {
    if !svc.auto_recall {
        return WhsBeforeRunResult {
            messages: Vec::new(),
        };
    }

    // Extract the last user message as the recall query.
    let query = match history
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| &m.content)
    {
        Some(q) if q.len() >= 10 => q.clone(),
        _ => {
            return WhsBeforeRunResult {
                messages: Vec::new(),
            };
        }
    };

    let lance = &svc.lance;
    let mut lines = Vec::new();

    // Embed the user message once; reuse for entities + journals.
    let vector = match svc.embed(&query).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("auto-recall embed failed: {e}");
            return WhsBeforeRunResult {
                messages: Vec::new(),
            };
        }
    };

    // Semantic entity search.
    let entities = lance
        .search_entities_semantic(&vector, None, 5)
        .await
        .unwrap_or_default();
    for e in &entities {
        lines.push(format!("[{}] {}: {}", e.entity_type, e.key, e.value));
    }

    // 1-hop connections for top-3 matched entities.
    for e in entities.iter().take(3) {
        if let Ok(rels) = lance
            .find_connections(&e.id, None, Direction::Both, 5)
            .await
        {
            for r in &rels {
                let line = format!("{} -[{}]-> {}", r.source, r.relation, r.target);
                if !lines.contains(&line) {
                    lines.push(line);
                }
            }
        }
    }

    // Semantic journal search (reuse same embedding vector).
    if let Ok(journals) = lance.search_journals(&vector, agent, 2).await {
        for j in &journals {
            let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| j.created_at.to_string());
            let summary = truncate_utf8(&j.summary, 300);
            lines.push(format!("[journal {ts}] {summary}"));
        }
    }

    if lines.is_empty() {
        return WhsBeforeRunResult {
            messages: Vec::new(),
        };
    }

    let block = format!("<recall>\n{}\n</recall>", lines.join("\n"));
    WhsBeforeRunResult {
        messages: vec![SimpleMessage {
            role: "user".to_owned(),
            content: block,
        }],
    }
}

/// Handle a ServiceQuery — JSON-encoded query for list/search operations.
///
/// Supported query types (JSON):
/// - `{"op": "entities", "entity_type": "...", "limit": N}`
/// - `{"op": "relations", "entity_id": "...", "limit": N}`
/// - `{"op": "journals", "agent": "...", "limit": N}`
/// - `{"op": "search", "query": "...", "entity_type": "...", "limit": N}`
async fn handle_service_query(svc: &MemoryService, query: &str) -> String {
    let parsed: serde_json::Value = match serde_json::from_str(query) {
        Ok(v) => v,
        Err(e) => return format!("invalid query JSON: {e}"),
    };

    let op = parsed["op"].as_str().unwrap_or("");
    let default_limit = 50usize;

    match op {
        "entities" => {
            let entity_type = parsed["entity_type"].as_str();
            let limit = parsed["limit"]
                .as_u64()
                .map(|l| l as usize)
                .unwrap_or(default_limit);
            match svc.lance.list_entities(entity_type, limit).await {
                Ok(entities) => {
                    let items: Vec<serde_json::Value> = entities
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "entity_type": e.entity_type,
                                "key": e.key,
                                "value": e.value,
                                "created_at": e.created_at,
                            })
                        })
                        .collect();
                    serde_json::to_string(&items)
                        .unwrap_or_else(|e| format!("serialize error: {e}"))
                }
                Err(e) => format!("entities query failed: {e}"),
            }
        }
        "relations" => {
            let entity_id = parsed["entity_id"].as_str();
            let limit = parsed["limit"]
                .as_u64()
                .map(|l| l as usize)
                .unwrap_or(default_limit);
            match svc.lance.list_relations(entity_id, limit).await {
                Ok(relations) => {
                    let items: Vec<serde_json::Value> = relations
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "source": r.source,
                                "relation": r.relation,
                                "target": r.target,
                                "created_at": r.created_at,
                            })
                        })
                        .collect();
                    serde_json::to_string(&items)
                        .unwrap_or_else(|e| format!("serialize error: {e}"))
                }
                Err(e) => format!("relations query failed: {e}"),
            }
        }
        "journals" => {
            let agent = parsed["agent"].as_str();
            let limit = parsed["limit"]
                .as_u64()
                .map(|l| l as usize)
                .unwrap_or(default_limit);
            match svc.lance.list_journals(agent, limit).await {
                Ok(journals) => {
                    let items: Vec<serde_json::Value> = journals
                        .iter()
                        .map(|j| {
                            serde_json::json!({
                                "summary": j.summary,
                                "agent": j.agent,
                                "created_at": j.created_at,
                            })
                        })
                        .collect();
                    serde_json::to_string(&items)
                        .unwrap_or_else(|e| format!("serialize error: {e}"))
                }
                Err(e) => format!("journals query failed: {e}"),
            }
        }
        "search" => {
            let query_str = parsed["query"].as_str().unwrap_or("");
            let entity_type = parsed["entity_type"].as_str();
            let limit = parsed["limit"]
                .as_u64()
                .map(|l| l as usize)
                .unwrap_or(default_limit);
            match svc
                .lance
                .search_entities(query_str, entity_type, limit)
                .await
            {
                Ok(entities) => {
                    let items: Vec<serde_json::Value> = entities
                        .iter()
                        .map(|e| {
                            serde_json::json!({
                                "entity_type": e.entity_type,
                                "key": e.key,
                                "value": e.value,
                                "created_at": e.created_at,
                            })
                        })
                        .collect();
                    serde_json::to_string(&items)
                        .unwrap_or_else(|e| format!("serialize error: {e}"))
                }
                Err(e) => format!("search query failed: {e}"),
            }
        }
        _ => format!("unknown op: '{op}'. supported: entities, relations, journals, search"),
    }
}
