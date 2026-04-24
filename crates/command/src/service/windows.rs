//! Windows service management via Task Scheduler (`schtasks.exe`).

use anyhow::Result;
use wcore::paths::LOGS_DIR;

/// Embedded Task Scheduler XML template.
pub const TEMPLATE: &str = include_str!("schtasks.xml");

/// Check if a scheduled task with the given label exists.
pub fn is_installed(label: &str) -> bool {
    std::process::Command::new("schtasks")
        .args(["/Query", "/TN", label])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Create and start a scheduled task from a rendered XML template.
pub fn install(rendered: &str, label: &str) -> Result<()> {
    // End any running instance before re-creating.
    // /Create /F already overwrites the task definition, so no need to delete first.
    if is_installed(label) {
        let _ = std::process::Command::new("schtasks")
            .args(["/End", "/TN", label])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    std::fs::create_dir_all(&*LOGS_DIR)?;

    // Write XML to a temp file — schtasks /Create /XML requires a file path.
    let xml_path = std::env::temp_dir().join(format!("{}.xml", label.replace('.', "_")));
    std::fs::write(&xml_path, rendered)?;

    let output = std::process::Command::new("schtasks")
        .args(["/Create", "/TN", label, "/XML"])
        .arg(&xml_path)
        .arg("/F")
        .output()?;

    // Clean up temp file regardless of outcome.
    let _ = std::fs::remove_file(&xml_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("schtasks /Create failed: {stderr}");
    }

    // Start the task immediately (logon trigger only fires at next login).
    let status = std::process::Command::new("schtasks")
        .args(["/Run", "/TN", label])
        .status()?;
    if status.success() {
        println!("service created and started");
    } else {
        println!("service created (will start at next logon)");
    }
    Ok(())
}

/// Stop and delete a scheduled task.
pub fn uninstall(label: &str) -> Result<()> {
    if !is_installed(label) {
        anyhow::bail!("service not installed ({label})");
    }

    // End the running instance first.
    let _ = std::process::Command::new("schtasks")
        .args(["/End", "/TN", label])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let status = std::process::Command::new("schtasks")
        .args(["/Delete", "/TN", label, "/F"])
        .status()?;
    if !status.success() {
        eprintln!("warning: schtasks /Delete exited with {status}");
    }

    println!("service uninstalled");
    Ok(())
}
