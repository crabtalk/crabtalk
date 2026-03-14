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
    model::Tool,
    protocol::{
        PROTOCOL_VERSION,
        codec::{read_message, write_message},
        whs::{
            Capability, ToolsList, WhsError, WhsHello, WhsReady, WhsRegisterTools, WhsRequest,
            WhsResponse, WhsToolSchemas, capability, whs_request, whs_response,
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
}

const HANDSHAKE_TIMEOUT: time::Duration = time::Duration::from_secs(10);

impl ServiceManager {
    /// Create a new manager from config. Does not spawn anything yet.
    pub fn new(configs: &BTreeMap<String, ServiceConfig>, config_dir: &Path) -> Self {
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
        }
    }

    /// Spawn all enabled services. Hook services get `--socket <path>` appended.
    pub async fn spawn_all(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.services_dir).context("create services dir")?;

        for (name, entry) in &mut self.entries {
            // Clean up stale socket.
            if entry.socket_path.exists() {
                let _ = std::fs::remove_file(&entry.socket_path);
            }

            let mut cmd = tokio::process::Command::new(&entry.config.command);
            cmd.args(&entry.config.args);

            // Hook services get the socket path so they can bind it.
            if matches!(entry.config.kind, ServiceKind::Hook) {
                cmd.arg("--socket").arg(&entry.socket_path);
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

            match self.handshake_one(name, &entry.socket_path).await {
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
