//! macOS launchd service management.

use anyhow::Result;
use wcore::paths::LOGS_DIR;

/// Embedded launchd service template.
pub const TEMPLATE: &str = include_str!("launchd.plist");

fn launchctl_domain() -> String {
    let uid = std::process::Command::new("id")
        .arg("-u")
        .output()
        .expect("failed to run `id -u`");
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    format!("gui/{uid}")
}

pub fn is_installed(label: &str) -> bool {
    dirs::home_dir()
        .map(|h| {
            h.join(format!("Library/LaunchAgents/{label}.plist"))
                .exists()
        })
        .unwrap_or(false)
}

pub fn install(rendered: &str, label: &str) -> Result<()> {
    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(format!("Library/LaunchAgents/{label}.plist"));

    if plist_path.exists() {
        uninstall(label)?;
    }

    std::fs::create_dir_all(&*LOGS_DIR)?;

    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&plist_path, rendered)?;
    println!("wrote {}", plist_path.display());

    let output = std::process::Command::new("launchctl")
        .args(["bootstrap", &launchctl_domain()])
        .arg(&plist_path)
        .output()?;
    if output.status.success() {
        println!("service loaded and started");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("launchctl bootstrap failed: {stderr}");
    }
    Ok(())
}

pub fn uninstall(label: &str) -> Result<()> {
    let plist_path = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
        .join(format!("Library/LaunchAgents/{label}.plist"));

    if !plist_path.exists() {
        anyhow::bail!("service not installed ({})", plist_path.display());
    }

    let service_target = format!("{}/{label}", launchctl_domain());
    let status = std::process::Command::new("launchctl")
        .args(["bootout", &service_target])
        .status()?;
    if !status.success() {
        eprintln!("warning: launchctl bootout exited with {}", status);
    }

    std::fs::remove_file(&plist_path)?;
    println!("service uninstalled");
    Ok(())
}
