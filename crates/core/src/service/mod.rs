//! Shared system service management (launchd/systemd).

use crate::paths::{CONFIG_DIR, LOGS_DIR, RUN_DIR};
use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

// ── Embedded templates ──────────────────────────────────────────────

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");

// ── Service trait ───────────────────────────────────────────────────

/// Trait for external command binaries that run as system services.
///
/// Implementors provide metadata; `start`/`stop`/`logs` come free.
pub trait Service {
    /// Service name, e.g. "search". Used for log/port files.
    fn name(&self) -> &str;
    /// Human description.
    fn description(&self) -> &str;
    /// Reverse-DNS label, e.g. "ai.crabtalk.search".
    fn label(&self) -> &str;
    /// CLI subcommand prefix used in the service template.
    fn subcommand(&self) -> &str;

    /// Install and start the service.
    fn start(&self) -> anyhow::Result<()> {
        let binary = std::env::current_exe()?;
        let rendered = render_service_template(self, &binary);
        install(&rendered, self.label())
    }

    /// Stop and uninstall the service.
    fn stop(&self) -> anyhow::Result<()> {
        uninstall(self.label())?;
        let port_file = RUN_DIR.join(format!("{}.port", self.name()));
        let _ = std::fs::remove_file(&port_file);
        Ok(())
    }

    /// View service logs.
    fn logs(&self, tail_args: &[String]) -> anyhow::Result<()> {
        view_logs(self.name(), tail_args)
    }
}

/// Render the platform-specific service template for a [`Service`] implementor.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn render_service_template(svc: &(impl Service + ?Sized), binary: &Path) -> String {
    let path_env = std::env::var("PATH").unwrap_or_default();
    #[cfg(target_os = "macos")]
    let template = LAUNCHD_TEMPLATE;
    #[cfg(target_os = "linux")]
    let template = SYSTEMD_TEMPLATE;
    template
        .replace("{label}", svc.label())
        .replace("{description}", svc.description())
        .replace("{subcommand}", svc.subcommand())
        .replace("{log_name}", svc.name())
        .replace("{binary}", &binary.display().to_string())
        .replace("{logs_dir}", &LOGS_DIR.display().to_string())
        .replace("{config_dir}", &CONFIG_DIR.display().to_string())
        .replace("{path}", &path_env)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn render_service_template(_svc: &(impl Service + ?Sized), _binary: &Path) -> String {
    String::new()
}

// ── Low-level install/uninstall ─────────────────────────────────────

#[cfg(target_os = "macos")]
pub use macos::{install, uninstall};

#[cfg(target_os = "linux")]
pub use linux::{install, uninstall};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install(_rendered: &str, _label: &str) -> anyhow::Result<()> {
    anyhow::bail!("service install is only supported on macOS and Linux")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall(_label: &str) -> anyhow::Result<()> {
    anyhow::bail!("service uninstall is only supported on macOS and Linux")
}

/// View service logs by delegating to `tail`.
///
/// `log_name` corresponds to the `{log_name}.log` file under `~/.crabtalk/logs/`.
/// Extra args (e.g. `-f`, `-n 100`) are passed through to `tail`.
/// Defaults to `-n 50` if no extra args are given.
pub fn view_logs(log_name: &str, tail_args: &[String]) -> anyhow::Result<()> {
    let path = LOGS_DIR.join(format!("{log_name}.log"));
    if !path.exists() {
        anyhow::bail!("log file not found: {}", path.display());
    }

    let args = if tail_args.is_empty() {
        vec!["-n".to_owned(), "50".to_owned()]
    } else {
        tail_args.to_vec()
    };

    let status = std::process::Command::new("tail")
        .args(&args)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run tail: {e}"))?;
    if !status.success() {
        anyhow::bail!("tail exited with {status}");
    }
    Ok(())
}

// ── ServiceAction (clap enum) ───────────────────────────────────────

#[cfg(any(feature = "mcp", feature = "client"))]
#[derive(Debug, clap::Subcommand)]
pub enum ServiceAction {
    /// Install and start the service.
    Start,
    /// Stop and uninstall the service.
    Stop,
    /// Run the service directly (used by launchd/systemd).
    Run,
    /// View service logs.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
}

#[cfg(any(feature = "mcp", feature = "client"))]
impl ServiceAction {
    /// Dispatch the action for an MCP (port-bound) service.
    #[cfg(feature = "mcp")]
    pub async fn exec_mcp(self, svc: &(impl McpService + Sync)) -> anyhow::Result<()> {
        match self {
            Self::Start => svc.start(),
            Self::Stop => svc.stop(),
            Self::Run => {
                let router = svc.router();
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let addr = listener.local_addr()?;
                std::fs::create_dir_all(&*RUN_DIR)?;
                std::fs::write(
                    RUN_DIR.join(format!("{}.port", svc.name())),
                    addr.port().to_string(),
                )?;
                eprintln!("MCP server listening on {addr}");
                axum::serve(listener, router).await?;
                Ok(())
            }
            Self::Logs { tail_args } => svc.logs(&tail_args),
        }
    }

    /// Dispatch the action for a client (daemon-connected) service.
    #[cfg(feature = "client")]
    pub async fn exec_client(self, svc: &(impl ClientService + Sync)) -> anyhow::Result<()> {
        match self {
            Self::Start => svc.start(),
            Self::Stop => svc.stop(),
            Self::Run => svc.run().await,
            Self::Logs { tail_args } => svc.logs(&tail_args),
        }
    }
}

// ── McpService ──────────────────────────────────────────────────────

/// MCP (port-bound) service. Implementors provide an axum Router.
#[cfg(feature = "mcp")]
pub trait McpService: Service {
    /// Return the axum Router for the MCP server.
    fn router(&self) -> axum::Router;
}

// ── ClientService ───────────────────────────────────────────────────

/// Client (daemon-connected) service. Implementors provide a run method.
#[cfg(feature = "client")]
pub trait ClientService: Service {
    /// Run the client service.
    fn run(&self) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}
