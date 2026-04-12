//! System service management (install/uninstall daemon as launchd/systemd service).

use anyhow::Result;
use wcore::paths::{CONFIG_DIR, LOGS_DIR};

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");
#[cfg(target_os = "windows")]
const SCHTASKS_TEMPLATE: &str = include_str!("schtasks.xml");

const LABEL: &str = "ai.crabtalk.crabtalk";

fn render_daemon_template(template: &str, verbose: u8) -> Result<String> {
    let binary = std::env::current_exe()?;
    let path_env = std::env::var("PATH").unwrap_or_default();
    Ok(template
        .replace("{label}", LABEL)
        .replace("{description}", "Crabtalk Daemon")
        .replace("{log_name}", "daemon")
        .replace("{binary}", &binary.display().to_string())
        .replace("-v", &command::verbose_flag(verbose))
        .replace("{logs_dir}", &LOGS_DIR.display().to_string())
        .replace("{config_dir}", &CONFIG_DIR.display().to_string())
        .replace("{path}", &path_env))
}

#[cfg(target_os = "macos")]
pub fn install(verbose: u8, force: bool) -> Result<()> {
    if !force && command::is_installed(LABEL) {
        println!("daemon is already running");
        return Ok(());
    }
    let rendered = render_daemon_template(LAUNCHD_TEMPLATE, verbose)?;
    command::install(&rendered, LABEL)
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    command::uninstall(LABEL)
}

#[cfg(target_os = "linux")]
pub fn install(verbose: u8, force: bool) -> Result<()> {
    if !force && command::is_installed(LABEL) {
        println!("daemon is already running");
        return Ok(());
    }
    let rendered = render_daemon_template(SYSTEMD_TEMPLATE, verbose)?;
    command::install(&rendered, LABEL)
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    command::uninstall(LABEL)
}

#[cfg(target_os = "windows")]
pub fn install(verbose: u8, force: bool) -> Result<()> {
    if !force && command::is_installed(LABEL) {
        println!("daemon is already running");
        return Ok(());
    }
    let rendered = render_daemon_template(SCHTASKS_TEMPLATE, verbose)?;
    command::install(&rendered, LABEL)
}

#[cfg(target_os = "windows")]
pub fn uninstall() -> Result<()> {
    command::uninstall(LABEL)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn install(_verbose: u8, _force: bool) -> Result<()> {
    anyhow::bail!("daemon start is not supported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub fn uninstall() -> Result<()> {
    anyhow::bail!("daemon stop is not supported on this platform")
}
