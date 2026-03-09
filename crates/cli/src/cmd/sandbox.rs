//! Workspace sandbox management commands.
//!
//! Creates and manages a dedicated OS user for agent isolation.
//! The runtime has zero sandbox logic — the operating system enforces
//! boundaries via standard Unix file permissions and ACLs.

use anyhow::{Result, anyhow, bail};
use clap::{Args, Subcommand};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Manage the workspace sandbox.
#[derive(Args, Debug)]
pub struct Sandbox {
    /// Sandbox subcommand.
    #[command(subcommand)]
    pub command: SandboxCommand,
}

/// Sandbox subcommands.
#[derive(Subcommand, Debug)]
pub enum SandboxCommand {
    /// Create the walrus system user and home directory structure.
    Init,
    /// Grant the walrus user access to a host resource via ACLs.
    Share {
        /// Path to the resource to share.
        path: PathBuf,
        /// Grant read-only access instead of read-write.
        #[arg(long)]
        read_only: bool,
        /// Copy the resource instead of sharing via ACL.
        #[arg(long, requires = "into")]
        copy: bool,
        /// Destination name within the walrus workspaces directory.
        /// Must match `[a-zA-Z0-9_-]+`.
        #[arg(long)]
        into: Option<String>,
    },
    /// Revoke the walrus user's ACL access to a host resource.
    Unshare {
        /// Path to the resource to unshare.
        path: PathBuf,
    },
    /// List sandbox status and shared resources.
    Shared,
}

impl Sandbox {
    /// Run the sandbox command.
    pub async fn run(self) -> Result<()> {
        match self.command {
            SandboxCommand::Init => init(),
            SandboxCommand::Share {
                path,
                read_only,
                copy,
                into,
            } => share(&path, read_only, copy, into.as_deref()),
            SandboxCommand::Unshare { path } => unshare(&path),
            SandboxCommand::Shared => shared(),
        }
    }
}

/// The system username for sandbox isolation.
const WALRUS_USER: &str = "walrus";

/// Check if running as root.
fn is_root() -> bool {
    std::env::var("USER").as_deref() == Ok("root")
}

/// Check if the walrus user already exists via `id walrus`.
fn user_exists() -> bool {
    Command::new("id")
        .arg(WALRUS_USER)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Resolve the home directory path for the walrus user.
fn walrus_home() -> &'static str {
    if cfg!(target_os = "macos") {
        "/Users/walrus"
    } else {
        "/home/walrus"
    }
}

/// Run a command, printing it first. Returns error with stderr on failure.
fn run_cmd(cmd: &str, args: &[&str]) -> Result<()> {
    let display = format!("{cmd} {}", args.join(" "));
    println!("  > {display}");
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| anyhow!("failed to run `{cmd}`: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "`{display}` failed (exit {}): {}",
            output.status,
            stderr.trim()
        );
    }
}

/// `walrus sandbox init` — create the walrus system user and directories.
fn init() -> Result<()> {
    // Must be root.
    if !is_root() {
        bail!("walrus sandbox init requires root. Re-run with: sudo walrus sandbox init");
    }

    // Check if user already exists.
    if user_exists() {
        println!("User `{WALRUS_USER}` already exists.");
        ensure_directories()?;
        println!("Sandbox ready.");
        return Ok(());
    }

    println!("Creating system user `{WALRUS_USER}`...");
    if cfg!(target_os = "macos") {
        create_user_macos()?;
    } else {
        create_user_linux()?;
    }

    ensure_directories()?;
    println!("Sandbox ready.");
    Ok(())
}

/// Create the walrus user on macOS using `dscl`.
fn create_user_macos() -> Result<()> {
    let home = walrus_home();
    let uid = next_available_uid_macos()?;
    let uid_str = uid.to_string();
    let user_path = format!("/Users/{WALRUS_USER}");

    run_cmd("dscl", &[".", "-create", &user_path])?;
    run_cmd(
        "dscl",
        &[".", "-create", &user_path, "UserShell", "/bin/zsh"],
    )?;
    run_cmd("dscl", &[".", "-create", &user_path, "UniqueID", &uid_str])?;
    run_cmd(
        "dscl",
        &[".", "-create", &user_path, "PrimaryGroupID", "20"],
    )?;
    run_cmd(
        "dscl",
        &[".", "-create", &user_path, "NFSHomeDirectory", home],
    )?;
    run_cmd(
        "dscl",
        &[".", "-create", &user_path, "RealName", "Walrus Agent"],
    )?;

    println!("  Created user `{WALRUS_USER}` (UID {uid}).");
    Ok(())
}

/// Find the next available UID >= 300 on macOS (service account range).
fn next_available_uid_macos() -> Result<u32> {
    let output = Command::new("dscl")
        .args([".", "-list", "/Users", "UniqueID"])
        .output()
        .map_err(|e| anyhow!("failed to list UIDs: {e}"))?;
    if !output.status.success() {
        bail!("dscl list UIDs failed");
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut used: Vec<u32> = stdout
        .lines()
        .filter_map(|line| line.split_whitespace().last()?.parse().ok())
        .collect();
    used.sort_unstable();

    let mut candidate = 300u32;
    for uid in &used {
        if *uid == candidate {
            candidate += 1;
        } else if *uid > candidate {
            break;
        }
    }
    Ok(candidate)
}

/// Create the walrus user on Linux using `useradd`.
fn create_user_linux() -> Result<()> {
    let home = walrus_home();
    run_cmd(
        "useradd",
        &[
            "--system",
            "--create-home",
            "--home-dir",
            home,
            "--shell",
            "/bin/bash",
            WALRUS_USER,
        ],
    )?;
    println!("  Created user `{WALRUS_USER}`.");
    Ok(())
}

/// Ensure the home directory structure exists with correct ownership.
fn ensure_directories() -> Result<()> {
    let home = walrus_home();
    let workspaces = format!("{home}/workspaces");
    let runtimes = format!("{home}/.runtimes");

    for dir in [home, workspaces.as_str(), runtimes.as_str()] {
        if !Path::new(dir).exists() {
            println!("  Creating {dir}");
            run_cmd("mkdir", &["-p", dir])?;
        }
    }

    // Ensure ownership — trailing colon resolves to user's primary group.
    run_cmd("chown", &["-R", &format!("{WALRUS_USER}:"), home])?;

    Ok(())
}

// ── Share / Unshare ─────────────────────────────────────────────────

/// Validate that a destination name matches `[a-zA-Z0-9_-]+`.
fn validate_dest(dest: &str) -> Result<()> {
    if dest.is_empty()
        || !dest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        bail!("destination name must match [a-zA-Z0-9_-]+, got: {dest}");
    }
    Ok(())
}

/// `walrus sandbox share` — grant ACL access or copy a resource.
fn share(path: &Path, read_only: bool, copy: bool, into: Option<&str>) -> Result<()> {
    if !user_exists() {
        bail!("walrus user does not exist. Run `sudo walrus sandbox init` first.");
    }

    let path = path
        .canonicalize()
        .map_err(|e| anyhow!("cannot resolve path {}: {e}", path.display()))?;

    if copy {
        let dest = into.expect("--into is required with --copy");
        validate_dest(dest)?;
        copy_resource(&path, dest)
    } else {
        grant_acl(&path, read_only)
    }
}

/// Grant ACL access to the walrus user for the given path.
fn grant_acl(path: &Path, read_only: bool) -> Result<()> {
    let path_str = path.to_string_lossy();
    let mode = if read_only { "read-only" } else { "read-write" };

    if cfg!(target_os = "macos") {
        let perm = if read_only {
            format!("{WALRUS_USER} allow read,readattr,readextattr,readsecurity,list,search")
        } else {
            format!(
                "{WALRUS_USER} allow \
                 read,write,append,readattr,writeattr,readextattr,writeextattr,\
                 readsecurity,list,search,add_file,add_subdirectory,delete_child"
            )
        };
        run_cmd("chmod", &["-R", "+a", &perm, &path_str])?;
    } else {
        let perm = if read_only {
            format!("{WALRUS_USER}:r-X")
        } else {
            format!("{WALRUS_USER}:rwX")
        };
        run_cmd("setfacl", &["-R", "-m", &format!("u:{perm}"), &path_str])?;
        // Set default ACL for new files in directories.
        if path.is_dir() {
            run_cmd("setfacl", &["-R", "-m", &format!("d:u:{perm}"), &path_str])?;
        }
    }

    println!("Shared {path_str} with {WALRUS_USER} ({mode}).");
    Ok(())
}

/// Copy a resource into the walrus workspaces directory.
fn copy_resource(path: &Path, dest: &str) -> Result<()> {
    let target = format!("{}/workspaces/{dest}", walrus_home());
    let path_str = path.to_string_lossy();

    if Path::new(&target).exists() {
        bail!("destination already exists: {target}");
    }

    // Use cp with reflink=auto for COW on APFS/Btrfs.
    if cfg!(target_os = "macos") {
        // macOS cp uses -c for clonefile (COW).
        run_cmd("cp", &["-Rc", &path_str, &target])?;
    } else {
        run_cmd("cp", &["-R", "--reflink=auto", &path_str, &target])?;
    }

    // Set ownership to walrus.
    run_cmd("chown", &["-R", &format!("{WALRUS_USER}:"), &target])?;

    println!("Copied {path_str} → {target}");
    Ok(())
}

/// `walrus sandbox unshare` — revoke ACL access for the walrus user.
fn unshare(path: &Path) -> Result<()> {
    if !user_exists() {
        bail!("walrus user does not exist. Run `sudo walrus sandbox init` first.");
    }

    let path = path
        .canonicalize()
        .map_err(|e| anyhow!("cannot resolve path {}: {e}", path.display()))?;
    let path_str = path.to_string_lossy();

    if cfg!(target_os = "macos") {
        let perm = format!(
            "{WALRUS_USER} allow \
             read,write,append,readattr,writeattr,readextattr,writeextattr,\
             readsecurity,list,search,add_file,add_subdirectory,delete_child"
        );
        run_cmd("chmod", &["-R", "-a", &perm, &path_str])?;
    } else {
        run_cmd(
            "setfacl",
            &["-R", "-x", &format!("u:{WALRUS_USER}"), &path_str],
        )?;
        if path.is_dir() {
            run_cmd(
                "setfacl",
                &["-R", "-x", &format!("d:u:{WALRUS_USER}"), &path_str],
            )?;
        }
    }

    println!("Unshared {path_str} from {WALRUS_USER}.");
    Ok(())
}

// ── Status / List Shared ────────────────────────────────────────────

/// `walrus sandbox shared` — show sandbox status and list shared resources.
fn shared() -> Result<()> {
    // Check sandbox status.
    if !user_exists() {
        println!("Sandbox: not initialized");
        println!("  Run `sudo walrus sandbox init` to create the walrus user.");
        return Ok(());
    }

    let home = walrus_home();
    println!("Sandbox: initialized");
    println!("  User: {WALRUS_USER}");
    println!("  Home: {home}");

    // List copied resources in workspaces/.
    let workspaces = format!("{home}/workspaces");
    let ws_path = Path::new(&workspaces);
    if ws_path.exists() {
        let mut entries: Vec<_> = std::fs::read_dir(ws_path)
            .map_err(|e| anyhow!("cannot read {workspaces}: {e}"))?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|ft| ft.is_dir()))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        if !entries.is_empty() {
            println!("\nCopied resources:");
            for entry in &entries {
                println!("  {}/", entry.path().display());
            }
        }
    }

    Ok(())
}
