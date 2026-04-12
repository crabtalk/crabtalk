//! External command launcher with auto-install.
//!
//! When `crabtalk <name>` is called with an unrecognized subcommand, this
//! module resolves `crabtalk-<name>`, auto-installs from crates.io if missing,
//! and forwards all arguments directly.

use anyhow::{Context, Result, bail};
use std::{ffi::OsString, io::Write, path::PathBuf, process::Command};

/// Resolve and launch an external `crabtalk-<name>` binary.
pub fn run(args: Vec<OsString>) -> Result<()> {
    let name = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("no subcommand provided"))?
        .to_string_lossy()
        .to_string();
    let bin_name = format!("crabtalk-{name}");

    let binary = match find_binary(&bin_name) {
        Some(path) => path,
        None => {
            let auto_approve = if !has_cargo() {
                if !confirm(&format!(
                    "{bin_name} requires cargo. Install Rust toolchain and {bin_name}?"
                ))? {
                    bail!("installation cancelled");
                }
                install_rustup()?;
                true
            } else {
                false
            };
            if !auto_approve
                && !confirm(&format!(
                    "{bin_name} is not installed. Install from crates.io?"
                ))?
            {
                bail!("installation cancelled");
            }
            eprintln!("installing {bin_name} from crates.io...");
            let status = Command::new("cargo")
                .args(["install", &bin_name])
                .status()
                .context("failed to run cargo install")?;
            if !status.success() {
                bail!("package crabtalk-{name} not found on crates.io");
            }
            find_binary(&bin_name)
                .ok_or_else(|| anyhow::anyhow!("{bin_name} not found after install"))?
        }
    };

    let status = Command::new(&binary)
        .args(&args[1..])
        .status()
        .with_context(|| format!("failed to run {}", binary.display()))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Install Rust via rustup.
fn install_rustup() -> Result<()> {
    eprintln!("installing Rust via rustup...");

    #[cfg(unix)]
    let status = Command::new("sh")
        .args([
            "-c",
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
        ])
        .status()
        .context("failed to run rustup installer")?;

    #[cfg(windows)]
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Invoke-WebRequest -Uri https://win.rustup.rs/x86_64 -OutFile rustup-init.exe; ./rustup-init.exe -y; Remove-Item rustup-init.exe",
        ])
        .status()
        .context("failed to run rustup installer")?;

    if !status.success() {
        bail!("rustup installation failed");
    }

    // Add cargo bin to PATH so it's available for the rest of this process.
    if let Some(home) = dirs::home_dir() {
        let cargo_bin = home.join(".cargo").join("bin");
        if cargo_bin.exists() {
            let path = std::env::var("PATH").unwrap_or_default();
            let sep = if cfg!(windows) { ";" } else { ":" };
            unsafe { std::env::set_var("PATH", format!("{}{sep}{path}", cargo_bin.display())) }
        }
    }

    if !has_cargo() {
        bail!("cargo not found after rustup install");
    }
    eprintln!("Rust installed successfully");
    Ok(())
}

/// Check if cargo is available on PATH or in `~/.cargo/bin`.
fn has_cargo() -> bool {
    find_binary("cargo").is_some()
}

/// Prompt the user for yes/no confirmation.
fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim(), "y" | "Y"))
}

/// Look for an external binary next to the current exe, then on PATH,
/// then in `~/.cargo/bin` as a fallback.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Ok(current) = std::env::current_exe()
        && let Some(dir) = current.parent()
    {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Fallback: ~/.cargo/bin may not be on PATH yet.
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".cargo/bin").join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}
