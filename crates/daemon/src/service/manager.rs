//! Service lifecycle management — spawn, handshake, registry, shutdown.

use crate::service::config::{ServiceConfig, ServiceKind};
use anyhow::{Context, Result, bail};
use compact_str::CompactString;
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
    AgentConfig, Hook, ToolRegistry,
    model::{Message, Role, Tool},
    protocol::{
        PROTOCOL_VERSION,
        codec::{read_message, write_message},
        whs::{
            Capability, SimpleMessage, ToolsList, WhsBeforeRun, WhsBeforeRunResult, WhsBuildAgent,
            WhsBuildAgentResult, WhsCompact, WhsCompactResult, WhsConfigure, WhsConfigured,
            WhsError, WhsEvent, WhsHello, WhsReady, WhsRegisterTools, WhsRequest, WhsResponse,
            WhsToolCall, WhsToolResult, WhsToolSchemas, capability, whs_request, whs_response,
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
}

impl ServiceRegistry {
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

impl Hook for ServiceRegistry {
    fn on_build_agent(&self, config: AgentConfig) -> AgentConfig {
        let mut config = config;
        let additions = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut additions = Vec::new();
                for handle in &self.build_agent {
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
                    match time::timeout(std::time::Duration::from_secs(10), handle.request(&req))
                        .await
                    {
                        Ok(Ok(resp)) => {
                            if let Some(whs_response::Msg::BuildAgentResult(WhsBuildAgentResult {
                                prompt_addition,
                                ..
                            })) = resp.msg
                                && !prompt_addition.is_empty()
                            {
                                additions.push(prompt_addition);
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                service = %handle.name, error = %e,
                                "BuildAgent dispatch failed"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                service = %handle.name,
                                "BuildAgent dispatch timeout"
                            );
                        }
                    }
                }
                additions
            })
        });
        for addition in additions {
            config.system_prompt.push_str(&addition);
        }
        config
    }

    fn on_compact(&self, prompt: &mut String) {
        let prompt_clone = prompt.clone();
        let additions = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut additions = Vec::new();
                for handle in &self.compact {
                    let req = WhsRequest {
                        msg: Some(whs_request::Msg::Compact(WhsCompact {
                            agent: String::new(),
                            prompt: prompt_clone.clone(),
                        })),
                    };
                    match time::timeout(std::time::Duration::from_secs(10), handle.request(&req))
                        .await
                    {
                        Ok(Ok(resp)) => {
                            if let Some(whs_response::Msg::CompactResult(WhsCompactResult {
                                addition,
                            })) = resp.msg
                                && !addition.is_empty()
                            {
                                additions.push(addition);
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                service = %handle.name, error = %e,
                                "Compact dispatch failed"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                service = %handle.name,
                                "Compact dispatch timeout"
                            );
                        }
                    }
                }
                additions
            })
        });
        for addition in additions {
            prompt.push_str(&addition);
        }
    }

    fn on_before_run(&self, agent: &str, history: &[Message]) -> Vec<Message> {
        let agent = agent.to_owned();
        let simple_history: Vec<SimpleMessage> = history
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
            .collect();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut messages = Vec::new();
                for handle in &self.before_run {
                    let req = WhsRequest {
                        msg: Some(whs_request::Msg::BeforeRun(WhsBeforeRun {
                            agent: agent.clone(),
                            history: simple_history.clone(),
                        })),
                    };
                    match time::timeout(std::time::Duration::from_secs(10), handle.request(&req))
                        .await
                    {
                        Ok(Ok(resp)) => {
                            if let Some(whs_response::Msg::BeforeRunResult(WhsBeforeRunResult {
                                messages: whs_msgs,
                            })) = resp.msg
                            {
                                for sm in whs_msgs {
                                    let msg = if sm.role == "assistant" {
                                        Message::assistant(sm.content, None, None)
                                    } else {
                                        Message::user(sm.content)
                                    };
                                    messages.push(msg);
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                service = %handle.name, error = %e,
                                "BeforeRun dispatch failed"
                            );
                        }
                        Err(_) => {
                            tracing::warn!(
                                service = %handle.name,
                                "BeforeRun dispatch timeout"
                            );
                        }
                    }
                }
                messages
            })
        })
    }

    async fn on_register_tools(&self, tools: &mut ToolRegistry) {
        tools.insert_all(self.tool_schemas.clone());
    }
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
    /// Daemon UDS socket path — passed to client services via `--daemon`.
    daemon_socket: PathBuf,
}

const HANDSHAKE_TIMEOUT: time::Duration = time::Duration::from_secs(10);

impl ServiceManager {
    /// Create a new manager from config. Does not spawn anything yet.
    ///
    /// `daemon_socket` is the daemon's UDS path — forwarded to client services
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
    /// Client services get `--daemon <path>` and `--config <json>` so they
    /// can connect back to the daemon.
    pub async fn spawn_all(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.services_dir).context("create services dir")?;

        for (name, entry) in &mut self.entries {
            // Clean up stale socket.
            if entry.socket_path.exists() {
                let _ = std::fs::remove_file(&entry.socket_path);
            }

            let mut cmd = tokio::process::Command::new(&entry.config.command);
            cmd.args(&entry.config.args);

            match entry.config.kind {
                ServiceKind::Hook => {
                    cmd.arg("--socket").arg(&entry.socket_path);
                }
                ServiceKind::Client => {
                    cmd.arg("--daemon").arg(&self.daemon_socket);
                    let config_json = serde_json::to_string(&entry.config.config)
                        .unwrap_or_else(|_| "{}".to_owned());
                    cmd.arg("--config").arg(config_json);
                }
                ServiceKind::Process => {}
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
