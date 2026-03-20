//! Shared system service management (launchd/systemd).

use crate::paths::{CONFIG_DIR, LOGS_DIR};
use std::path::Path;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

/// Parameters for rendering a service template.
pub struct ServiceParams<'a> {
    pub label: &'a str,
    pub description: &'a str,
    pub subcommand: &'a str,
    pub log_name: &'a str,
    pub binary: &'a Path,
    pub socket: &'a Path,
    pub config_path: &'a Path,
}

/// Render a template by replacing placeholder tokens with values from `params`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn render_template(template: &str, params: &ServiceParams<'_>) -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    template
        .replace("{label}", params.label)
        .replace("{description}", params.description)
        .replace("{subcommand}", params.subcommand)
        .replace("{log_name}", params.log_name)
        .replace("{binary}", &params.binary.display().to_string())
        .replace("{socket}", &params.socket.display().to_string())
        .replace("{config_path}", &params.config_path.display().to_string())
        .replace("{logs_dir}", &LOGS_DIR.display().to_string())
        .replace("{config_dir}", &CONFIG_DIR.display().to_string())
        .replace("{path}", &path)
}

#[cfg(target_os = "macos")]
pub use macos::{install, uninstall};

#[cfg(target_os = "linux")]
pub use linux::{install, uninstall};

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install(_template: &str, _params: &ServiceParams<'_>) -> anyhow::Result<()> {
    anyhow::bail!("service install is only supported on macOS and Linux")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall(_params: &ServiceParams<'_>) -> anyhow::Result<()> {
    anyhow::bail!("service uninstall is only supported on macOS and Linux")
}

/// View service logs by delegating to `tail`.
///
/// `log_name` corresponds to the `{log_name}.log` file under `~/.crabtalk/logs/`.
/// Extra args (e.g. `-f`, `-n 100`) are passed through to `tail`.
/// Defaults to `-n 50` if no extra args are given.
pub fn logs(log_name: &str, tail_args: &[String]) -> anyhow::Result<()> {
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
