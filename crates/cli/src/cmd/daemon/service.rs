//! `walrus daemon install/uninstall` — system service management.

use anyhow::Result;
use std::path::Path;
use wcore::paths::CONFIG_DIR;

#[cfg(target_os = "macos")]
const LAUNCHD_TEMPLATE: &str = include_str!("launchd.plist");
#[cfg(target_os = "linux")]
const SYSTEMD_TEMPLATE: &str = include_str!("systemd.service");

/// Render a template by replacing `{binary}` and `{log_dir}` placeholders.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn render_template(template: &str, binary: &Path, log_dir: &Path) -> String {
    template
        .replace("{binary}", &binary.display().to_string())
        .replace("{log_dir}", &log_dir.display().to_string())
}

#[cfg(target_os = "macos")]
pub fn install() -> Result<()> {
    let binary = std::env::current_exe()?;
    let plist = render_template(LAUNCHD_TEMPLATE, &binary, &CONFIG_DIR);

    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join("Library/LaunchAgents/xyz.openwalrus.walrus.plist");

    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist_path, plist)?;
    println!("wrote {}", plist_path.display());

    let status = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(&plist_path)
        .status()?;
    if status.success() {
        println!("service loaded and started");
    } else {
        anyhow::bail!("launchctl load failed (exit {})", status);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join("Library/LaunchAgents/xyz.openwalrus.walrus.plist");

    if !plist_path.exists() {
        anyhow::bail!("service not installed ({})", plist_path.display());
    }

    let status = std::process::Command::new("launchctl")
        .args(["unload", "-w"])
        .arg(&plist_path)
        .status()?;
    if !status.success() {
        eprintln!("warning: launchctl unload exited with {}", status);
    }

    std::fs::remove_file(&plist_path)?;
    println!("service uninstalled");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn install() -> Result<()> {
    let binary = std::env::current_exe()?;
    let unit = render_template(SYSTEMD_TEMPLATE, &binary, &CONFIG_DIR);

    let unit_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;

    let unit_path = unit_dir.join("walrus-daemon.service");
    std::fs::write(&unit_path, unit)?;
    println!("wrote {}", unit_path.display());

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", "walrus-daemon.service"])
        .status()?;
    if status.success() {
        println!("service enabled and started");
    } else {
        anyhow::bail!("systemctl enable failed (exit {})", status);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    let unit_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user/walrus-daemon.service");

    if !unit_path.exists() {
        anyhow::bail!("service not installed ({})", unit_path.display());
    }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", "walrus-daemon.service"])
        .status()?;
    if !status.success() {
        eprintln!("warning: systemctl disable exited with {}", status);
    }

    std::fs::remove_file(&unit_path)?;
    println!("service uninstalled");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install() -> Result<()> {
    anyhow::bail!("service install is only supported on macOS and Linux")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall() -> Result<()> {
    anyhow::bail!("service uninstall is only supported on macOS and Linux")
}
