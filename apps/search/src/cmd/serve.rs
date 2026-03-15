//! WHS serve command — run walrus-search as a hook service over UDS.

use crate::{
    aggregator::Aggregator,
    browser::fetch,
    config::Config,
    tool::{WebFetch, WebSearch, tool_defs},
};
use std::path::Path;
use wcore::protocol::{
    PROTOCOL_VERSION,
    codec::{read_message, write_message},
    whs::{
        Capability, ToolsList, WhsConfigured, WhsError, WhsReady, WhsRequest, WhsResponse,
        WhsToolResult, WhsToolSchemas, capability, whs_request, whs_response,
    },
};

pub async fn run(socket: &Path) -> anyhow::Result<()> {
    // Clean up stale socket from a previous run.
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    let listener = tokio::net::UnixListener::bind(socket)?;
    tracing::info!("search service listening on {}", socket.display());

    let (stream, _) = listener.accept().await?;
    let (mut reader, mut writer) = stream.into_split();

    // Hello → Ready
    let hello: WhsRequest = read_message(&mut reader).await?;
    match hello.msg {
        Some(whs_request::Msg::Hello(_)) => {}
        other => anyhow::bail!("expected Hello, got {other:?}"),
    }
    let ready = WhsResponse {
        msg: Some(whs_response::Msg::Ready(WhsReady {
            version: PROTOCOL_VERSION.to_owned(),
            service: "search".to_owned(),
            capabilities: vec![Capability {
                cap: Some(capability::Cap::Tools(ToolsList {
                    names: vec!["web_search".to_owned(), "web_fetch".to_owned()],
                })),
            }],
        })),
    };
    write_message(&mut writer, &ready).await?;

    // Configure → Configured
    let configure: WhsRequest = read_message(&mut reader).await?;
    let config = match configure.msg {
        Some(whs_request::Msg::Configure(c)) => {
            if c.config.is_empty() {
                Config::default()
            } else {
                serde_json::from_str(&c.config).unwrap_or_else(|e| {
                    tracing::warn!("invalid config, using defaults: {e}");
                    Config::default()
                })
            }
        }
        other => anyhow::bail!("expected Configure, got {other:?}"),
    };
    let configured = WhsResponse {
        msg: Some(whs_response::Msg::Configured(WhsConfigured {})),
    };
    write_message(&mut writer, &configured).await?;

    // RegisterTools → ToolSchemas
    let register: WhsRequest = read_message(&mut reader).await?;
    match register.msg {
        Some(whs_request::Msg::RegisterTools(_)) => {}
        other => anyhow::bail!("expected RegisterTools, got {other:?}"),
    }
    let schemas = WhsResponse {
        msg: Some(whs_response::Msg::ToolSchemas(WhsToolSchemas {
            tools: tool_defs(),
        })),
    };
    write_message(&mut writer, &schemas).await?;
    tracing::info!("handshake complete");

    // Build runtime resources from config.
    let aggregator = Aggregator::new(config.clone())?;
    let fetch_client = fetch::default_client()?;

    // Dispatch loop
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
                let result = dispatch(&call.name, &call.args, &aggregator, &fetch_client).await;
                WhsResponse {
                    msg: Some(whs_response::Msg::ToolResult(WhsToolResult { result })),
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

async fn dispatch(
    name: &str,
    args: &str,
    aggregator: &Aggregator,
    fetch_client: &reqwest::Client,
) -> String {
    match name {
        "web_search" => {
            let input: WebSearch = match serde_json::from_str(args) {
                Ok(v) => v,
                Err(e) => return format!("invalid arguments: {e}"),
            };
            let max = input.max_results.unwrap_or(10);
            match aggregator.search(&input.query, 0).await {
                Ok(mut results) => {
                    results.results.truncate(max);
                    let count = results.results.len();
                    let mut out = format!(
                        "Found {count} results for \"{}\" ({}ms):\n",
                        results.query, results.elapsed_ms
                    );
                    for (i, r) in results.results.iter().enumerate() {
                        out.push_str(&format!(
                            "\n{}. {}\n   {}\n   {}\n",
                            i + 1,
                            r.title,
                            r.url,
                            r.description
                        ));
                    }
                    if !results.engine_errors.is_empty() {
                        out.push_str("\nEngine errors:\n");
                        for e in &results.engine_errors {
                            out.push_str(&format!("  - {}: {}\n", e.engine, e.error));
                        }
                    }
                    out
                }
                Err(e) => format!("web_search failed: {e}"),
            }
        }
        "web_fetch" => {
            let input: WebFetch = match serde_json::from_str(args) {
                Ok(v) => v,
                Err(e) => return format!("invalid arguments: {e}"),
            };
            match fetch::fetch_url(&input.url, fetch_client).await {
                Ok(result) => {
                    format!(
                        "# {}\nURL: {}\nContent-Length: {}\n\n{}",
                        result.title, result.url, result.content_length, result.content
                    )
                }
                Err(e) => format!("web_fetch failed: {e}"),
            }
        }
        _ => format!("unknown tool: {name}"),
    }
}
