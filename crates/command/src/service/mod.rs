//! Shared system service management (launchd/systemd).

use std::path::Path;
use wcore::paths::{CONFIG_DIR, LOGS_DIR, RUN_DIR};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod unknown;

// ── Low-level install/uninstall ─────────────────────────────────────

#[cfg(target_os = "macos")]
pub use macos::{TEMPLATE, install, is_installed, uninstall};

#[cfg(target_os = "linux")]
pub use linux::{TEMPLATE, install, is_installed, uninstall};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub use unknown::{install, is_installed, uninstall};

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

    /// Install and start the service.
    ///
    /// If the service is already installed, prints a message and returns
    /// unless `force` is set, in which case it re-installs.
    fn start(&self, force: bool) -> anyhow::Result<()> {
        if !force && is_installed(self.label()) {
            println!("{} is already running", self.name());
            return Ok(());
        }
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

/// Build the `-v`/`-vv`/`-vvv` flag string from a count (empty when 0).
pub fn verbose_flag(count: u8) -> String {
    if count == 0 {
        String::new()
    } else {
        format!("-{}", "v".repeat(count as usize))
    }
}

/// Render the platform-specific service template for a [`Service`] implementor.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn render_service_template(svc: &(impl Service + ?Sized), binary: &Path) -> String {
    let path_env = std::env::var("PATH").unwrap_or_default();
    TEMPLATE
        .replace("{label}", svc.label())
        .replace("{description}", svc.description())
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

// ── MCP run helper ──────────────────────────────────────────────────

/// Run an MCP (port-bound) service: bind a TCP listener, write the port file,
/// and serve the router.
#[cfg(feature = "mcp")]
pub async fn run_mcp(svc: &(impl McpService + Sync)) -> anyhow::Result<()> {
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

// ── McpService ──────────────────────────────────────────────────────

/// MCP (port-bound) service. Implementors provide an axum Router.
#[cfg(feature = "mcp")]
pub trait McpService: Service {
    /// Return the axum Router for the MCP server.
    fn router(&self) -> axum::Router;
}
