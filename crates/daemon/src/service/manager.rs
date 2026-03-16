//! Service lifecycle management — spawn, handshake, registry, shutdown.

use crate::service::config::{ServiceConfig, ServiceKind};
use anyhow::{Context, Result, bail};
use compact_str::CompactString;
use model::ProviderManager;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    net::unix::{OwnedReadHalf, OwnedWriteHalf},
    process::Child,
    sync::Mutex,
    time,
};
use wcore::{
    AgentConfig, CompactHook, Hook, ToolRegistry,
    model::{Message, Model, Request, Role, Tool},
    protocol::{
        PROTOCOL_VERSION,
        codec::{read_message, write_message},
        whs::{
            Capability, SimpleMessage, ToolsList, WhsAfterRun, WhsBeforeRun, WhsBeforeRunResult,
            WhsBuildAgent, WhsBuildAgentResult, WhsCompact, WhsCompactResult, WhsConfigure,
            WhsConfigured, WhsError, WhsEvent, WhsHello, WhsInferResult, WhsReady,
            WhsRegisterTools, WhsRequest, WhsResponse, WhsToolCall, WhsToolResult, WhsToolSchemas,
            capability, whs_request, whs_response,
        },
    },
};

/// Handle to a connected hook service.
pub struct ServiceHandle {
    pub name: CompactString,
    pub capabilities: Vec<Capability>,
    writer: Mutex<OwnedWriteHalf>,
    reader: Mutex<OwnedReadHalf>,
    /// Serializes request-response pairs to prevent interleaving.
    rpc_lock: Mutex<()>,
}

impl ServiceHandle {
    /// Send a WHS request and read one response.
    pub async fn request(&self, req: &WhsRequest) -> Result<WhsResponse> {
        let _guard = self.rpc_lock.lock().await;
        let mut w = self.writer.lock().await;
        write_message(&mut *w, req).await.context("whs write")?;
        drop(w);
        let mut r = self.reader.lock().await;
        let resp: WhsResponse = read_message(&mut *r).await.context("whs read")?;
        Ok(resp)
    }

    /// Send a fire-and-forget WHS request (no response expected).
    pub async fn send(&self, req: &WhsRequest) -> Result<()> {
        let _guard = self.rpc_lock.lock().await;
        let mut w = self.writer.lock().await;
        write_message(&mut *w, req).await.context("whs write")?;
        Ok(())
    }
}

/// Capability-indexed runtime state built during handshake.
#[derive(Default)]
pub struct ServiceRegistry {
    /// Tool name → owning service handle.
    pub tools: BTreeMap<String, Arc<ServiceHandle>>,
    /// Service name → handle (for ServiceQuery routing).
    pub query: BTreeMap<String, Arc<ServiceHandle>>,
    /// Tool schemas collected from all hook services.
    pub tool_schemas: Vec<Tool>,
    /// Services that declared BuildAgent capability.
    pub build_agent: Vec<Arc<ServiceHandle>>,
    /// Services that declared BeforeRun capability.
    pub before_run: Vec<Arc<ServiceHandle>>,
    /// Services that declared Compact capability.
    pub compact: Vec<Arc<ServiceHandle>>,
    /// Services that declared EventObserver capability.
    pub event_observer: Vec<Arc<ServiceHandle>>,
    /// Services that declared AfterRun capability.
    pub after_run: Vec<Arc<ServiceHandle>>,
    /// Model for Infer fulfillment (set after runtime construction).
    model: Option<ProviderManager>,
}

impl ServiceRegistry {
    /// Set the model for Infer fulfillment.
    pub fn set_model(&mut self, model: ProviderManager) {
        self.model = Some(model);
    }

    /// Fire-and-forget event to all EventObserver services.
    pub async fn fire_event(&self, agent: &str, event: &str) {
        let req = WhsRequest {
            msg: Some(whs_request::Msg::Event(WhsEvent {
                agent: agent.to_owned(),
                event: event.to_owned(),
            })),
        };
        for handle in &self.event_observer {
            if let Err(e) = handle.send(&req).await {
                tracing::warn!(
                    service = %handle.name, error = %e,
                    "Event dispatch failed"
                );
            }
        }
    }

    /// Dispatch a tool call to the owning WHS service.
    /// Returns `None` if the tool is not in the registry.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        task_id: Option<u64>,
    ) -> Option<String> {
        let handle = self.tools.get(name)?;
        let req = WhsRequest {
            msg: Some(whs_request::Msg::ToolCall(WhsToolCall {
                name: name.to_owned(),
                args: args.to_owned(),
                agent: agent.to_owned(),
                task_id,
            })),
        };
        Some(
            match time::timeout(std::time::Duration::from_secs(30), handle.request(&req)).await {
                Ok(Ok(resp)) => match resp.msg {
                    Some(whs_response::Msg::ToolResult(WhsToolResult { result })) => result,
                    Some(whs_response::Msg::Error(WhsError { message })) => {
                        format!("service error: {message}")
                    }
                    other => format!("unexpected response: {other:?}"),
                },
                Ok(Err(e)) => format!("service unavailable: {name} ({e})"),
                Err(_) => format!("service timeout: {name}"),
            },
        )
    }
}

/// Send a WHS request with timeout and uniform error logging.
/// Returns `Some(response)` on success, `None` on error or timeout.
async fn send_with_timeout(
    handle: &ServiceHandle,
    req: &WhsRequest,
    timeout_secs: u64,
    label: &str,
) -> Option<WhsResponse> {
    match time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        handle.request(req),
    )
    .await
    {
        Ok(Ok(resp)) => Some(resp),
        Ok(Err(e)) => {
            tracing::warn!(service = %handle.name, error = %e, "{label} dispatch failed");
            None
        }
        Err(_) => {
            tracing::warn!(service = %handle.name, "{label} dispatch timeout");
            None
        }
    }
}

/// Fan out a request to multiple service handles, collecting extracted results.
///
/// Builds the same request for each handle, sends with timeout, and calls
/// `extract` on successful responses. Skips handles that error or timeout.
async fn fan_out<T>(
    handles: &[Arc<ServiceHandle>],
    req: &WhsRequest,
    timeout_secs: u64,
    label: &str,
    extract: impl Fn(WhsResponse) -> Option<T>,
) -> Vec<T> {
    let mut results = Vec::new();
    for handle in handles {
        if let Some(resp) = send_with_timeout(handle, req, timeout_secs, label).await
            && let Some(val) = extract(resp)
        {
            results.push(val);
        }
    }
    results
}

impl Hook for ServiceRegistry {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        let mut config = config;
        let req = WhsRequest {
            msg: Some(whs_request::Msg::BuildAgent(WhsBuildAgent {
                name: config.name.to_string(),
                description: config.description.to_string(),
                system_prompt: config.system_prompt.clone(),
                tools: config.tools.iter().map(|t| t.to_string()).collect(),
                skills: config.skills.clone(),
                mcps: config.mcps.clone(),
                members: config.members.clone(),
            })),
        };
        let additions: Vec<String> = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fan_out(
                &self.build_agent,
                &req,
                10,
                "BuildAgent",
                |resp| match resp.msg {
                    Some(whs_response::Msg::BuildAgentResult(WhsBuildAgentResult {
                        prompt_addition,
                        ..
                    })) if !prompt_addition.is_empty() => Some(prompt_addition),
                    _ => None,
                },
            ))
        });
        for addition in additions {
            config.system_prompt.push_str(&addition);
        }
        config
    }

    fn on_compact(&self, agent: &str, prompt: &mut String) {
        let req = WhsRequest {
            msg: Some(whs_request::Msg::Compact(WhsCompact {
                agent: agent.to_owned(),
                prompt: prompt.clone(),
            })),
        };
        let additions: Vec<String> = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fan_out(
                &self.compact,
                &req,
                10,
                "Compact",
                |resp| match resp.msg {
                    Some(whs_response::Msg::CompactResult(WhsCompactResult { addition }))
                        if !addition.is_empty() =>
                    {
                        Some(addition)
                    }
                    _ => None,
                },
            ))
        });
        for addition in additions {
            prompt.push_str(&addition);
        }
    }

    fn on_before_run(&self, agent: &str, history: &[Message]) -> Vec<Message> {
        let req = WhsRequest {
            msg: Some(whs_request::Msg::BeforeRun(WhsBeforeRun {
                agent: agent.to_owned(),
                history: to_simple_messages(history),
            })),
        };
        let batches: Vec<Vec<SimpleMessage>> = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fan_out(
                &self.before_run,
                &req,
                10,
                "BeforeRun",
                |resp| match resp.msg {
                    Some(whs_response::Msg::BeforeRunResult(WhsBeforeRunResult { messages })) => {
                        Some(messages)
                    }
                    _ => None,
                },
            ))
        });
        batches
            .into_iter()
            .flatten()
            .map(|sm| match sm.role.as_str() {
                "assistant" => Message::assistant(sm.content, None, None),
                "system" => Message::system(&sm.content),
                _ => Message::user(sm.content),
            })
            .collect()
    }

    fn on_after_run(&self, agent: &str, history: &[Message], system_prompt: &str) {
        if self.after_run.is_empty() {
            return;
        }
        let simple_history = to_simple_messages(history);
        let agent = agent.to_owned();
        let system_prompt = system_prompt.to_owned();
        let model = self.model.clone();

        for handle in &self.after_run {
            let handle = Arc::clone(handle);
            // Filter tools to only those owned by this service.
            let service_tools: Arc<Vec<Tool>> = Arc::new(
                self.tool_schemas
                    .iter()
                    .filter(|t| {
                        self.tools
                            .get(t.name.as_str())
                            .is_some_and(|h| h.name == handle.name)
                    })
                    .cloned()
                    .collect(),
            );
            let tool_handles: Arc<BTreeMap<String, Arc<ServiceHandle>>> = Arc::new(
                self.tools
                    .iter()
                    .filter(|(_, h)| h.name == handle.name)
                    .map(|(k, v)| (k.clone(), Arc::clone(v)))
                    .collect(),
            );
            let agent = agent.clone();
            let history = simple_history.clone();
            let system_prompt = system_prompt.clone();
            let model = model.clone();
            tokio::spawn(async move {
                let req = WhsRequest {
                    msg: Some(whs_request::Msg::AfterRun(WhsAfterRun {
                        agent: agent.clone(),
                        history,
                        system_prompt: system_prompt.clone(),
                    })),
                };
                let Some(resp) = send_with_timeout(&handle, &req, 30, "AfterRun").await else {
                    return;
                };
                match resp.msg {
                    Some(whs_response::Msg::AfterRunResult(_)) => {
                        tracing::debug!(service = %handle.name, "AfterRun complete");
                    }
                    Some(whs_response::Msg::InferRequest(infer_req)) => {
                        if let Some(ref model) = model {
                            if let Err(e) = infer_fulfill(
                                model,
                                &handle,
                                &agent,
                                &system_prompt,
                                infer_req.messages,
                                &service_tools,
                                &tool_handles,
                            )
                            .await
                            {
                                tracing::warn!(
                                    service = %handle.name,
                                    error = %e,
                                    "Infer fulfillment failed"
                                );
                            }
                        } else {
                            tracing::warn!(
                                service = %handle.name,
                                "Infer requested but no model available"
                            );
                        }
                    }
                    Some(whs_response::Msg::Error(WhsError { message })) => {
                        tracing::warn!(
                            service = %handle.name,
                            error = %message,
                            "AfterRun service error"
                        );
                    }
                    other => {
                        tracing::warn!(
                            service = %handle.name,
                            "unexpected AfterRun response: {other:?}"
                        );
                    }
                }
            });
        }
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        tools.insert_all(self.tool_schemas.clone());
    }
}

impl CompactHook for ServiceRegistry {
    fn on_compact(&self, agent: &str, prompt: &mut String) {
        <Self as Hook>::on_compact(self, agent, prompt);
    }
}

/// Convert core `Message` history to proto `SimpleMessage` for WHS transport.
fn to_simple_messages(history: &[Message]) -> Vec<SimpleMessage> {
    history
        .iter()
        .map(|m| SimpleMessage {
            role: match m.role {
                Role::User => "user".to_owned(),
                Role::Assistant => "assistant".to_owned(),
                Role::System => "system".to_owned(),
                Role::Tool => "tool".to_owned(),
            },
            content: m.content.clone(),
        })
        .collect()
}

/// Infer fulfillment: mini agent loop using the host agent's model.
///
/// Takes the service's messages, builds an LLM request with the host agent's
/// model and system prompt, auto-attaches service's tools, loops until final text.
/// Tool calls are dispatched back to the owning service.
async fn infer_fulfill(
    model: &ProviderManager,
    handle: &ServiceHandle,
    agent: &str,
    system_prompt: &str,
    initial_messages: Vec<SimpleMessage>,
    service_tools: &[Tool],
    tool_handles: &BTreeMap<String, Arc<ServiceHandle>>,
) -> Result<()> {
    let model_name = model.active_model();

    // Convert SimpleMessage → Message.
    // If the service provides its own system message, use that instead of
    // the host agent's system prompt to avoid conflicting instructions.
    let has_system = initial_messages.iter().any(|m| m.role == "system");
    let mut messages: Vec<Message> = Vec::with_capacity(1 + initial_messages.len());
    if !has_system && !system_prompt.is_empty() {
        messages.push(Message::system(system_prompt));
    }
    for sm in &initial_messages {
        let msg = match sm.role.as_str() {
            "assistant" => Message::assistant(&sm.content, None, None),
            "system" => Message::system(&sm.content),
            _ => Message::user(&sm.content),
        };
        messages.push(msg);
    }

    // Service tools (pre-filtered by caller).
    let tools: Vec<Tool> = service_tools.to_vec();

    const MAX_INFER_ITERATIONS: usize = 10;
    for _ in 0..MAX_INFER_ITERATIONS {
        let request = Request::new(model_name.clone())
            .with_messages(messages.clone())
            .with_tools(tools.clone());

        let response = model.send(&request).await.context("infer LLM call")?;
        let msg = response
            .message()
            .ok_or_else(|| anyhow::anyhow!("no message in LLM response"))?;

        let tool_calls = msg.tool_calls.to_vec();
        messages.push(msg);

        if tool_calls.is_empty() {
            // Final text response — extract content and send InferResult.
            let content = messages
                .last()
                .map(|m| m.content.clone())
                .unwrap_or_default();
            let result_req = WhsRequest {
                msg: Some(whs_request::Msg::InferResult(WhsInferResult { content })),
            };
            // Read the final response from the service after sending InferResult.
            let _final_resp = time::timeout(
                std::time::Duration::from_secs(30),
                handle.request(&result_req),
            )
            .await
            .context("InferResult timeout")?
            .context("InferResult send")?;
            tracing::debug!(service = %handle.name, %agent, "Infer fulfillment complete");
            return Ok(());
        }

        // Dispatch tool calls back to the owning service.
        for tc in &tool_calls {
            let tool_name = tc.function.name.as_str();
            let tool_handle = tool_handles
                .get(tool_name)
                .ok_or_else(|| anyhow::anyhow!("tool '{tool_name}' not in registry"))?;
            let tool_req = WhsRequest {
                msg: Some(whs_request::Msg::ToolCall(WhsToolCall {
                    name: tool_name.to_owned(),
                    args: tc.function.arguments.clone(),
                    agent: agent.to_owned(),
                    task_id: None,
                })),
            };
            let tool_resp = time::timeout(
                std::time::Duration::from_secs(30),
                tool_handle.request(&tool_req),
            )
            .await
            .context("tool call timeout")?
            .context("tool call")?;
            let result = match tool_resp.msg {
                Some(whs_response::Msg::ToolResult(WhsToolResult { result })) => result,
                Some(whs_response::Msg::Error(WhsError { message })) => {
                    format!("service error: {message}")
                }
                other => format!("unexpected tool response: {other:?}"),
            };
            messages.push(Message::tool(result, tc.id.clone()));
        }
    }

    tracing::warn!(
        service = %handle.name,
        "Infer hit max iterations ({MAX_INFER_ITERATIONS})"
    );
    // Send an InferResult so the service doesn't deadlock waiting for a response.
    let result_req = WhsRequest {
        msg: Some(whs_request::Msg::InferResult(WhsInferResult {
            content: format!("infer fulfillment exceeded max iterations ({MAX_INFER_ITERATIONS})"),
        })),
    };
    let _ = time::timeout(
        std::time::Duration::from_secs(5),
        handle.request(&result_req),
    )
    .await;
    Ok(())
}

/// Entry tracking a spawned service process.
struct ServiceEntry {
    config: ServiceConfig,
    child: Option<Child>,
    socket_path: PathBuf,
}

/// Manages the lifecycle of daemon child services.
pub struct ServiceManager {
    entries: BTreeMap<String, ServiceEntry>,
    services_dir: PathBuf,
    /// Daemon UDS socket path — passed to gateway services via `--daemon`.
    daemon_socket: PathBuf,
}

const HANDSHAKE_TIMEOUT: time::Duration = time::Duration::from_secs(10);

impl ServiceManager {
    /// Create a new manager from config. Does not spawn anything yet.
    ///
    /// `daemon_socket` is the daemon's UDS path — forwarded to gateway services
    /// so they can connect back.
    pub fn new(
        configs: &BTreeMap<String, ServiceConfig>,
        config_dir: &Path,
        daemon_socket: PathBuf,
    ) -> Self {
        let services_dir = config_dir.join("services");
        let entries = configs
            .iter()
            .filter(|(_, c)| c.enabled)
            .map(|(name, config)| {
                let socket_path = services_dir.join(format!("{name}.sock"));
                (
                    name.clone(),
                    ServiceEntry {
                        config: config.clone(),
                        child: None,
                        socket_path,
                    },
                )
            })
            .collect();
        Self {
            entries,
            services_dir,
            daemon_socket,
        }
    }

    /// Spawn all enabled services.
    ///
    /// Hook services get `--socket <path>` so they bind a UDS listener.
    /// Gateway services get `--daemon <path>` and `--config <json>` so they
    /// can connect back to the daemon.
    pub async fn spawn_all(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.services_dir).context("create services dir")?;

        for (name, entry) in &mut self.entries {
            // Clean up stale socket.
            if entry.socket_path.exists() {
                let _ = std::fs::remove_file(&entry.socket_path);
            }

            let mut cmd = tokio::process::Command::new(&entry.config.krate);
            for (k, v) in &entry.config.env {
                cmd.env(k, v);
            }

            match entry.config.kind {
                ServiceKind::Hook => {
                    cmd.arg("--socket").arg(&entry.socket_path);
                }
                ServiceKind::Gateway => {
                    cmd.arg("--daemon").arg(&self.daemon_socket);
                    let config_json = serde_json::to_string(&entry.config.config)
                        .unwrap_or_else(|_| "{}".to_owned());
                    cmd.arg("--config").arg(config_json);
                }
            }

            cmd.kill_on_drop(true);
            let child = cmd
                .spawn()
                .with_context(|| format!("spawn service '{name}'"))?;
            tracing::info!(service = %name, pid = child.id(), "spawned service");
            entry.child = Some(child);
        }

        Ok(())
    }

    /// Connect to all hook services and perform the WHS handshake.
    /// Returns a `ServiceRegistry` with tool and query mappings.
    pub async fn handshake_all(&self) -> ServiceRegistry {
        let mut registry = ServiceRegistry::default();

        for (name, entry) in &self.entries {
            if !matches!(entry.config.kind, ServiceKind::Hook) {
                continue;
            }

            match self
                .handshake_one(name, &entry.socket_path, &entry.config.config)
                .await
            {
                Ok((handle, schemas)) => {
                    let handle = Arc::new(handle);
                    Self::register(&mut registry, &handle);
                    tracing::info!(
                        service = %name,
                        tools = schemas.len(),
                        "hook service registered"
                    );
                    registry.tool_schemas.extend(schemas);
                }
                Err(e) => {
                    tracing::warn!(service = %name, error = %e, "hook handshake failed, skipping");
                }
            }
        }

        registry
    }

    /// Perform WHS handshake with a single hook service.
    /// Returns the handle and its declared tool schemas.
    async fn handshake_one(
        &self,
        name: &str,
        socket_path: &Path,
        config: &serde_json::Value,
    ) -> Result<(ServiceHandle, Vec<Tool>)> {
        // Wait for socket file to appear (service may need startup time).
        let deadline = time::Instant::now() + HANDSHAKE_TIMEOUT;
        loop {
            if socket_path.exists() {
                break;
            }
            if time::Instant::now() >= deadline {
                bail!(
                    "socket not found after {}s: {}",
                    HANDSHAKE_TIMEOUT.as_secs(),
                    socket_path.display()
                );
            }
            time::sleep(time::Duration::from_millis(50)).await;
        }

        let stream = time::timeout(
            HANDSHAKE_TIMEOUT,
            tokio::net::UnixStream::connect(socket_path),
        )
        .await
        .context("connect timeout")?
        .context("connect")?;

        let (read_half, write_half) = stream.into_split();
        let writer = Mutex::new(write_half);
        let reader = Mutex::new(read_half);

        // Hello → Ready
        let hello = WhsRequest {
            msg: Some(whs_request::Msg::Hello(WhsHello {
                version: PROTOCOL_VERSION.to_owned(),
            })),
        };
        {
            let mut w = writer.lock().await;
            write_message(&mut *w, &hello)
                .await
                .context("write Hello")?;
        }
        let ready: WhsResponse = {
            let mut r = reader.lock().await;
            time::timeout(HANDSHAKE_TIMEOUT, read_message(&mut *r))
                .await
                .context("Ready timeout")?
                .context("read Ready")?
        };
        let (service, capabilities) = match ready.msg {
            Some(whs_response::Msg::Ready(WhsReady {
                service,
                capabilities,
                ..
            })) => (service, capabilities),
            Some(whs_response::Msg::Error(WhsError { message })) => {
                bail!("service error: {message}")
            }
            other => bail!("unexpected response to Hello: {other:?}"),
        };
        tracing::debug!(service = %service, "handshake Hello/Ready complete");

        let handle = ServiceHandle {
            name: CompactString::from(service.as_str()),
            capabilities,
            writer,
            reader,
            rpc_lock: Mutex::new(()),
        };

        // Configure → Configured
        let config_json = serde_json::to_string(config).context("serialize service config")?;
        let configure_req = WhsRequest {
            msg: Some(whs_request::Msg::Configure(WhsConfigure {
                config: config_json,
            })),
        };
        let configure_resp = time::timeout(HANDSHAKE_TIMEOUT, handle.request(&configure_req))
            .await
            .context("Configure timeout")?
            .context("Configure")?;
        match configure_resp.msg {
            Some(whs_response::Msg::Configured(WhsConfigured {})) => {}
            Some(whs_response::Msg::Error(WhsError { message })) => {
                bail!("Configure error: {message}")
            }
            other => bail!("unexpected response to Configure: {other:?}"),
        }
        tracing::debug!(service = %name, "handshake Configure/Configured complete");

        // RegisterTools → ToolSchemas
        let register_tools_req = WhsRequest {
            msg: Some(whs_request::Msg::RegisterTools(WhsRegisterTools {})),
        };
        let resp = time::timeout(HANDSHAKE_TIMEOUT, handle.request(&register_tools_req))
            .await
            .context("RegisterTools timeout")?
            .context("RegisterTools")?;
        let tool_defs = match resp.msg {
            Some(whs_response::Msg::ToolSchemas(WhsToolSchemas { tools })) => tools,
            Some(whs_response::Msg::Error(WhsError { message })) => {
                bail!("RegisterTools error: {message}")
            }
            other => bail!("unexpected response to RegisterTools: {other:?}"),
        };
        tracing::debug!(service = %name, tools = tool_defs.len(), "handshake RegisterTools/ToolSchemas complete");

        // Convert ToolDef (proto) → Tool (domain).
        let tools: Vec<Tool> = tool_defs
            .into_iter()
            .map(|td| Tool {
                name: CompactString::from(td.name.as_str()),
                description: CompactString::from(td.description.as_str()),
                parameters: serde_json::from_slice(&td.parameters).unwrap_or_else(|_| true.into()),
                strict: td.strict,
            })
            .collect();

        Ok((handle, tools))
    }

    /// Populate the registry from a service handle's capabilities and tool schemas.
    fn register(registry: &mut ServiceRegistry, handle: &Arc<ServiceHandle>) {
        for cap in &handle.capabilities {
            match &cap.cap {
                Some(capability::Cap::Tools(ToolsList { names })) => {
                    for tool_name in names {
                        registry.tools.insert(tool_name.clone(), Arc::clone(handle));
                    }
                }
                Some(capability::Cap::Query(_)) => {
                    registry
                        .query
                        .insert(handle.name.to_string(), Arc::clone(handle));
                }
                Some(capability::Cap::BuildAgent(_)) => {
                    registry.build_agent.push(Arc::clone(handle));
                }
                Some(capability::Cap::BeforeRun(_)) => {
                    registry.before_run.push(Arc::clone(handle));
                }
                Some(capability::Cap::Compact(_)) => {
                    registry.compact.push(Arc::clone(handle));
                }
                Some(capability::Cap::EventObserver(_)) => {
                    registry.event_observer.push(Arc::clone(handle));
                }
                Some(capability::Cap::AfterRun(_)) => {
                    registry.after_run.push(Arc::clone(handle));
                }
                Some(capability::Cap::Infer(_)) => {
                    // Response-side capability — not stored in registry.
                }
                None => {}
            }
        }
    }

    /// Graceful shutdown of all services. Signals each child to stop,
    /// waits up to 5s, then force-kills stragglers.
    pub async fn shutdown_all(&mut self) {
        // Signal all children to stop.
        for (name, entry) in &mut self.entries {
            if let Some(ref mut child) = entry.child {
                tracing::debug!(service = %name, pid = child.id(), "stopping service");
                let _ = child.start_kill();
            }
        }

        // Wait for exit, force-kill on timeout.
        for (name, entry) in &mut self.entries {
            if let Some(ref mut child) = entry.child {
                match time::timeout(time::Duration::from_secs(5), child.wait()).await {
                    Ok(Ok(status)) => {
                        tracing::debug!(service = %name, %status, "service exited");
                    }
                    Ok(Err(e)) => {
                        tracing::warn!(service = %name, error = %e, "error waiting for service");
                    }
                    Err(_) => {
                        tracing::warn!(service = %name, "service did not exit in 5s, killing");
                        let _ = child.kill().await;
                    }
                }
            }
            let _ = std::fs::remove_file(&entry.socket_path);
        }
    }
}
