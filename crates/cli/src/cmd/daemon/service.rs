//! `crabtalk daemon start/stop` — system service management.

use crate::cmd::attach::setup_provider;
use anyhow::Result;
use wcore::paths::{CONFIG_DIR, LOGS_DIR};

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");

const LABEL: &str = "ai.crabtalk.crabtalk";

/// Check if providers are configured; scaffold config and prompt if needed.
fn ensure_providers() -> Result<()> {
    let config_path = CONFIG_DIR.join("crab.toml");
    if !config_path.exists() {
        ::daemon::config::scaffold_config_dir(&CONFIG_DIR)?;
    }

    let config = ::daemon::DaemonConfig::load(&config_path)?;
    if config.provider.is_empty() {
        setup_provider(&config_path)?;
    }
    Ok(())
}

fn render_daemon_template(template: &str, verbose: u8) -> Result<String> {
    let binary = std::env::current_exe()?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    Ok(template
        .replace("{label}", LABEL)
        .replace("{description}", "Crabtalk Daemon")
        .replace("{log_name}", "daemon")
        .replace("{binary}", &binary.display().to_string())
        .replace("-v", &crabtalk_command::verbose_flag(verbose))
        .replace("{logs_dir}", &LOGS_DIR.display().to_string())
        .replace("{config_dir}", &CONFIG_DIR.display().to_string())
        .replace("{path}", &path_env))
}

#[cfg(target_os = "macos")]
pub fn install(verbose: u8) -> Result<()> {
    ensure_providers()?;
    let rendered = render_daemon_template(LAUNCHD_TEMPLATE, verbose)?;
    crabtalk_command::install(&rendered, LABEL)
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    crabtalk_command::uninstall(LABEL)
}

#[cfg(target_os = "linux")]
pub fn install(verbose: u8) -> Result<()> {
    ensure_providers()?;
    let rendered = render_daemon_template(SYSTEMD_TEMPLATE, verbose)?;
    crabtalk_command::install(&rendered, LABEL)
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    crabtalk_command::uninstall(LABEL)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install(_verbose: u8) -> Result<()> {
    anyhow::bail!("daemon start is only supported on macOS and Linux")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall() -> Result<()> {
    anyhow::bail!("daemon stop is only supported on macOS and Linux")
}
