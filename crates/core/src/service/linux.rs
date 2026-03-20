//! Linux systemd service management.

use crate::paths::LOGS_DIR;
use anyhow::Result;

pub fn install(rendered: &str, label: &str) -> Result<()> {
    let unit_name = format!("{label}.service");

    let unit_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    std::fs::create_dir_all(&*LOGS_DIR)?;

    let unit_path = unit_dir.join(&unit_name);
    std::fs::write(&unit_path, rendered)?;
    println!("wrote {}", unit_path.display());

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", &unit_name])
        .status()?;
    if status.success() {
        println!("service enabled and started");
    } else {
        anyhow::bail!("systemctl enable failed (exit {})", status);
    }
    Ok(())
}

pub fn uninstall(label: &str) -> Result<()> {
    let unit_name = format!("{label}.service");

    let unit_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(".config/systemd/user")
        .join(&unit_name);

    if !unit_path.exists() {
        anyhow::bail!("service not installed ({})", unit_path.display());
    }

    let status = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", &unit_name])
        .status()?;
    if !status.success() {
        eprintln!("warning: systemctl disable exited with {}", status);
    }

    std::fs::remove_file(&unit_path)?;
    println!("service uninstalled");
    Ok(())
}
