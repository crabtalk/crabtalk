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
    ToolRegistry,
    model::Tool,
    protocol::{
        PROTOCOL_VERSION,
        codec::{read_message, write_message},
        ext::{
            Capability, ExtConfigure, ExtConfigured, ExtError, ExtHello, ExtReady,
            ExtRegisterTools, ExtRequest, ExtResponse, ExtToolCall, ExtToolResult, ExtToolSchemas,
            ToolsList, capability, ext_request, ext_response,
        },
    },
};

/// Handle to a connected extension service.
pub struct ServiceHandle {
    pub name: CompactString,
    pub capabilities: Vec<Capability>,
    writer: Mutex<OwnedWriteHalf>,
    reader: Mutex<OwnedReadHalf>,
    /// Serializes request-response pairs to prevent interleaving.
    rpc_lock: Mutex<()>,
}

impl ServiceHandle {
    /// Send an extension request and read one response.
    pub async fn request(&self, req: &ExtRequest) -> Result<ExtResponse> {
        let _guard = self.rpc_lock.lock().await;
        let mut w = self.writer.lock().await;
        write_message(&mut *w, req).await.context("ext write")?;
        drop(w);
        let mut r = self.reader.lock().await;
        let resp: ExtResponse = read_message(&mut *r).await.context("ext read")?;
        Ok(resp)
    }

    /// Send a fire-and-forget extension request (no response expected).
    pub async fn send(&self, req: &ExtRequest) -> Result<()> {
        let _guard = self.rpc_lock.lock().await;
        let mut w = self.writer.lock().await;
        write_message(&mut *w, req).await.context("ext write")?;
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
    /// Tool schemas collected from all extension services.
    pub tool_schemas: Vec<Tool>,
}

impl ServiceRegistry {
    /// Dispatch a tool call to the owning extension service.
    /// Returns `None` if the tool is not in the registry.
    pub async fn dispatch_tool(
        &self,
        name: &str,
        args: &str,
        agent: &str,
        task_id: Option<u64>,
    ) -> Option<String> {
        let handle = self.tools.get(name)?;
        let req = ExtRequest {
            msg: Some(ext_request::Msg::ToolCall(ExtToolCall {
                name: name.to_owned(),
                args: args.to_owned(),
                agent: agent.to_owned(),
                task_id,
            })),
        };
        Some(
            match time::timeout(std::time::Duration::from_secs(30), handle.request(&req)).await {
                Ok(Ok(resp)) => match resp.msg {
                    Some(ext_response::Msg::ToolResult(ExtToolResult { result })) => result,
                    Some(ext_response::Msg::Error(ExtError { message })) => {
                        format!("service error: {message}")
                    }
                    other => format!("unexpected response: {other:?}"),
                },
                Ok(Err(e)) => format!("service unavailable: {name} ({e})"),
                Err(_) => format!("service timeout: {name}"),
            },
        )
    }

    /// Register tool schemas into the tool registry.
    pub async fn register_tools(&self, tools: &mut ToolRegistry) {
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
    /// Extension services get `--socket <path>` so they bind a UDS listener.
    /// Gateway services get `--daemon <path>` and `--config <json>` so they
    /// can connect back to the daemon.
    pub async fn spawn_all(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.services_dir).context("create services dir")?;
        let logs_dir = &*wcore::paths::LOGS_DIR;
        std::fs::create_dir_all(logs_dir).context("create logs dir")?;

        for (name, entry) in &mut self.entries {
            // Clean up stale socket.
            if entry.socket_path.exists() {
                let _ = std::fs::remove_file(&entry.socket_path);
            }

            // Resolve binary: try ~/.cargo/bin/<krate> first (launchd/systemd
            // don't inherit the user's shell PATH), fall back to bare name.
            let cargo_bin = std::env::var("HOME").ok().map(|h| {
                PathBuf::from(h)
                    .join(".cargo/bin")
                    .join(&entry.config.krate)
            });
            let binary = match cargo_bin {
                Some(ref p) if p.exists() => p.as_path(),
                _ => Path::new(&entry.config.krate),
            };
            tracing::info!(
                service = %name,
                binary = %binary.display(),
                kind = ?entry.config.kind,
                "spawning service"
            );
            let mut cmd = tokio::process::Command::new(binary);
            for (k, v) in &entry.config.env {
                cmd.env(k, v);
            }

            // Forward RUST_LOG so child services inherit the daemon's log level.
            if !entry.config.env.contains_key("RUST_LOG")
                && let Ok(rust_log) = std::env::var("RUST_LOG")
            {
                cmd.env("RUST_LOG", rust_log);
            }

            // Redirect stdout/stderr to per-service log files.
            let log_path = logs_dir.join(format!("{name}.log"));
            let log_file = std::fs::File::create(&log_path)
                .with_context(|| format!("create log file for '{name}'"))?;
            cmd.stdout(log_file.try_clone()?);
            cmd.stderr(log_file);

            cmd.arg("serve");
            match entry.config.kind {
                ServiceKind::Extension => {
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
            let child = cmd.spawn().with_context(|| {
                format!("spawn service '{name}' (binary: {})", binary.display())
            })?;
            tracing::info!(service = %name, pid = child.id(), log = %log_path.display(), "spawned service");
            entry.child = Some(child);
        }

        Ok(())
    }

    /// Connect to all extension services and perform the handshake.
    /// Returns a `ServiceRegistry` with tool and query mappings.
    pub async fn handshake_all(&self) -> ServiceRegistry {
        let mut registry = ServiceRegistry::default();

        for (name, entry) in &self.entries {
            if !matches!(entry.config.kind, ServiceKind::Extension) {
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
                        "extension registered"
                    );
                    registry.tool_schemas.extend(schemas);
                }
                Err(e) => {
                    tracing::warn!(service = %name, error = %e, "extension handshake failed, skipping");
                }
            }
        }

        registry
    }

    /// Perform handshake with a single extension service.
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
        let hello = ExtRequest {
            msg: Some(ext_request::Msg::Hello(ExtHello {
                version: PROTOCOL_VERSION.to_owned(),
            })),
        };
        {
            let mut w = writer.lock().await;
            write_message(&mut *w, &hello)
                .await
                .context("write Hello")?;
        }
        let ready: ExtResponse = {
            let mut r = reader.lock().await;
            time::timeout(HANDSHAKE_TIMEOUT, read_message(&mut *r))
                .await
                .context("Ready timeout")?
                .context("read Ready")?
        };
        let (service, capabilities) = match ready.msg {
            Some(ext_response::Msg::Ready(ExtReady {
                service,
                capabilities,
                ..
            })) => (service, capabilities),
            Some(ext_response::Msg::Error(ExtError { message })) => {
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
        let configure_req = ExtRequest {
            msg: Some(ext_request::Msg::Configure(ExtConfigure {
                config: config_json,
            })),
        };
        let configure_resp = time::timeout(HANDSHAKE_TIMEOUT, handle.request(&configure_req))
            .await
            .context("Configure timeout")?
            .context("Configure")?;
        match configure_resp.msg {
            Some(ext_response::Msg::Configured(ExtConfigured {})) => {}
            Some(ext_response::Msg::Error(ExtError { message })) => {
                bail!("Configure error: {message}")
            }
            other => bail!("unexpected response to Configure: {other:?}"),
        }
        tracing::debug!(service = %name, "handshake Configure/Configured complete");

        // RegisterTools → ToolSchemas
        let register_tools_req = ExtRequest {
            msg: Some(ext_request::Msg::RegisterTools(ExtRegisterTools {})),
        };
        let resp = time::timeout(HANDSHAKE_TIMEOUT, handle.request(&register_tools_req))
            .await
            .context("RegisterTools timeout")?
            .context("RegisterTools")?;
        let tool_defs = match resp.msg {
            Some(ext_response::Msg::ToolSchemas(ExtToolSchemas { tools })) => tools,
            Some(ext_response::Msg::Error(ExtError { message })) => {
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

    /// Populate the registry from a service handle's capabilities.
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
                _ => {}
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
