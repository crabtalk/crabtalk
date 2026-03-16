//! Extension serve command — run walrus-memory as an extension service over UDS.

use crate::{config::MemoryConfig, dispatch::MemoryService, tool};
use std::path::Path;
use wcore::protocol::{
    PROTOCOL_VERSION,
    codec::{read_message, write_message},
    ext::{
        AfterRunCap, BeforeRunCap, BuildAgentCap, Capability, CompactCap, EventObserverCap,
        ExtAfterRunResult, ExtBeforeRunResult, ExtBuildAgentResult, ExtCompactResult,
        ExtConfigured, ExtError, ExtInferRequest, ExtReady, ExtRequest, ExtResponse,
        ExtServiceQueryResult, ExtToolResult, ExtToolSchemas, InferCap, QueryCap, SimpleMessage,
        ToolsList, capability, ext_request, ext_response,
    },
};

const EXTRACT_PROMPT: &str = include_str!("../../prompts/extract.md");

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
    let hello: ExtRequest = read_message(&mut reader).await?;
    match hello.msg {
        Some(ext_request::Msg::Hello(_)) => {}
        other => anyhow::bail!("expected Hello, got {other:?}"),
    }

    let tool_names = vec!["recall".to_owned(), "extract".to_owned()];

    let ready = ExtResponse {
        msg: Some(ext_response::Msg::Ready(ExtReady {
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
                Capability {
                    cap: Some(capability::Cap::AfterRun(AfterRunCap {})),
                },
                Capability {
                    cap: Some(capability::Cap::Infer(InferCap {})),
                },
            ],
        })),
    };
    write_message(&mut writer, &ready).await?;

    // ── Configure → Configured ───────────────────────────────────────
    let configure: ExtRequest = read_message(&mut reader).await?;
    let config = match configure.msg {
        Some(ext_request::Msg::Configure(c)) => {
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
    let configured = ExtResponse {
        msg: Some(ext_response::Msg::Configured(ExtConfigured {})),
    };
    write_message(&mut writer, &configured).await?;

    // ── RegisterTools → ToolSchemas ──────────────────────────────────
    let register: ExtRequest = read_message(&mut reader).await?;
    match register.msg {
        Some(ext_request::Msg::RegisterTools(_)) => {}
        other => anyhow::bail!("expected RegisterTools, got {other:?}"),
    }

    // Build the memory service before constructing dynamic tool schemas.
    let memory_dir = wcore::paths::CONFIG_DIR.join("memory");
    let svc = MemoryService::open(&memory_dir, &config).await?;

    // All tools including internal `extract` (needed by infer_fulfill).
    // Agent-visible filtering happens via BuildAgent response (tool_defs).
    let tools = tool::all_tool_defs();
    let schemas = ExtResponse {
        msg: Some(ext_response::Msg::ToolSchemas(ExtToolSchemas { tools })),
    };
    write_message(&mut writer, &schemas).await?;
    tracing::info!("handshake complete");

    // ── Dispatch loop ────────────────────────────────────────────────
    let mut clean_exit = false;
    loop {
        let req: ExtRequest = match read_message(&mut reader).await {
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
            Some(ext_request::Msg::ToolCall(call)) => {
                let result = dispatch_tool(&svc, &call.name, &call.args, &call.agent).await;
                ExtResponse {
                    msg: Some(ext_response::Msg::ToolResult(ExtToolResult { result })),
                }
            }
            Some(ext_request::Msg::BuildAgent(ba)) => {
                let result =
                    handle_build_agent(&svc, &ba.name, &ba.description, &ba.system_prompt).await;
                ExtResponse {
                    msg: Some(ext_response::Msg::BuildAgentResult(result)),
                }
            }
            Some(ext_request::Msg::BeforeRun(br)) => {
                let result = handle_before_run(&svc, &br.history).await;
                ExtResponse {
                    msg: Some(ext_response::Msg::BeforeRunResult(result)),
                }
            }
            Some(ext_request::Msg::AfterRun(ar)) => {
                let conversation = build_conversation_summary(&ar.history);
                // Store a journal entry from the conversation for future compaction context.
                let _ = svc.dispatch_journal(&conversation, &ar.agent).await;
                let messages = extraction_messages_from(&conversation);
                // Respond with InferRequest — daemon runs extraction LLM loop,
                // dispatching recall/extract tool calls back to this service.
                ExtResponse {
                    msg: Some(ext_response::Msg::InferRequest(ExtInferRequest {
                        messages,
                    })),
                }
            }
            Some(ext_request::Msg::InferResult(_)) => {
                // Infer complete — extraction tool calls already dispatched.
                ExtResponse {
                    msg: Some(ext_response::Msg::AfterRunResult(ExtAfterRunResult {})),
                }
            }
            Some(ext_request::Msg::Compact(c)) => {
                let addition = handle_compact(&svc, &c.agent).await;
                ExtResponse {
                    msg: Some(ext_response::Msg::CompactResult(ExtCompactResult {
                        addition,
                    })),
                }
            }
            Some(ext_request::Msg::ServiceQuery(sq)) => {
                let result = handle_service_query(&svc, &sq.query).await;
                ExtResponse {
                    msg: Some(ext_response::Msg::ServiceQueryResult(
                        ExtServiceQueryResult { result },
                    )),
                }
            }
            Some(ext_request::Msg::Event(_)) => {
                // Fire-and-forget — no response expected.
                continue;
            }
            Some(ext_request::Msg::GetSchema(_)) => ExtResponse {
                msg: Some(ext_response::Msg::Error(ExtError {
                    message: "schema not yet implemented".into(),
                })),
            },
            Some(ext_request::Msg::Shutdown(_)) => {
                tracing::info!("shutdown requested");
                clean_exit = true;
                break;
            }
            other => ExtResponse {
                msg: Some(ext_response::Msg::Error(ExtError {
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

/// Dispatch a tool call to the appropriate MemoryService method.
async fn dispatch_tool(svc: &MemoryService, name: &str, args: &str, _agent: &str) -> String {
    match name {
        "recall" => svc.dispatch_recall(args).await,
        "extract" => svc.dispatch_extract(args).await,
        _ => format!("unknown tool: {name}"),
    }
}

/// Handle the BuildAgent lifecycle event.
///
/// Builds prompt additions: `<self>`, `<identity>`, `<profile>` blocks
/// plus the memory prompt. Returns agent-visible tools only.
async fn handle_build_agent(
    svc: &MemoryService,
    name: &str,
    description: &str,
    _system_prompt: &str,
) -> ExtBuildAgentResult {
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

    // Append memory prompt.
    buf.push_str(&format!("\n\n{}", MemoryService::memory_prompt()));

    ExtBuildAgentResult {
        prompt_addition: buf,
        tools: tool::tool_defs(),
    }
}

/// Handle the BeforeRun lifecycle event.
///
/// Auto-recalls relevant entities and graph connections based on
/// the last user message via unified semantic search.
async fn handle_before_run(svc: &MemoryService, history: &[SimpleMessage]) -> ExtBeforeRunResult {
    if !svc.auto_recall {
        return ExtBeforeRunResult {
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
            return ExtBeforeRunResult {
                messages: Vec::new(),
            };
        }
    };

    let result = match svc.unified_search(&query, 5).await {
        Some(r) => r,
        None => {
            return ExtBeforeRunResult {
                messages: Vec::new(),
            };
        }
    };

    let block = format!("<recall>\n{result}\n</recall>");
    ExtBeforeRunResult {
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

/// Handle the Compact lifecycle event — inject recent journals into the prompt.
async fn handle_compact(svc: &MemoryService, agent: &str) -> String {
    let mut addition = String::new();
    if let Ok(journals) = svc.lance.recent_journals(agent, 3).await
        && !journals.is_empty()
    {
        addition.push_str("\n\nRecent conversation journals (preserve key context):\n");
        for j in &journals {
            let ts = chrono::DateTime::from_timestamp(j.created_at as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_else(|| j.created_at.to_string());
            addition.push_str(&format!("- [{ts}] {}\n", j.summary));
        }
    }
    addition
}

/// Build a condensed conversation summary from history, skipping recall
/// injections and tool messages.
fn build_conversation_summary(history: &[SimpleMessage]) -> String {
    let mut conversation = String::new();
    for msg in history {
        let role = msg.role.as_str();
        if msg.content.starts_with("<recall>") || role == "tool" {
            continue;
        }
        conversation.push_str(&format!("[{role}] {}\n\n", msg.content));
    }
    conversation
}

/// Wrap a conversation summary into extraction messages for the Infer LLM.
fn extraction_messages_from(conversation: &str) -> Vec<SimpleMessage> {
    vec![
        SimpleMessage {
            role: "system".to_owned(),
            content: EXTRACT_PROMPT.to_owned(),
        },
        SimpleMessage {
            role: "user".to_owned(),
            content: format!("Extract memories from this conversation:\n\n{conversation}"),
        },
    ]
}
