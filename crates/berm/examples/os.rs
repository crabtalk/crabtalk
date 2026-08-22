//! Exercise the OS harness against a real directory.
//!
//! ```sh
//! cargo build --release -p berm-os --target riscv64imac-unknown-none-elf
//! cargo run --example os -p crabtalk-berm
//! ```

use anyhow::{Context, Result};
use berm::{Berm, Config, Engine};
use crabtalk_berm::{exec, fs as harness_fs};
use std::{fs, path::PathBuf};

const GUEST: &str = "target/riscv64imac-unknown-none-elf/{profile}/os";

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,harness=info"))
        .init();

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .context("no workspace root")?
        .to_path_buf();

    let profile = std::env::args().nth(1).unwrap_or_else(|| "release".into());
    let guest = GUEST.replace("{profile}", &profile);
    let elf = fs::read(workspace.join(&guest)).with_context(|| {
        format!("build the guest first: cargo build --release -p berm-os --target riscv64imac-unknown-none-elf ({guest})")
    })?;

    let sandbox = std::env::temp_dir().join("berm-os");
    let _ = fs::remove_dir_all(&sandbox);
    fs::create_dir_all(&sandbox)?;
    fs::write(sandbox.join("hello.txt"), "alpha\nbeta\ngamma\n")?;

    let engine = Engine::new(&Config::new())?;
    let harness = Berm::load(
        &engine,
        &elf,
        &[
            harness_fs::read(sandbox.clone()),
            harness_fs::write(sandbox.clone()),
            exec::run(sandbox.clone()),
        ],
    )?;

    println!("tools:    {:?}", harness.tools().collect::<Vec<_>>());
    println!("manifest: {:?}\n", harness.manifest());

    show(&harness, "read", r#"{"path":"hello.txt"}"#)?;
    show(
        &harness,
        "read",
        r#"{"path":"hello.txt","offset":2,"limit":1}"#,
    )?;
    show(
        &harness,
        "edit",
        r#"{"path":"hello.txt","old_string":"beta","new_string":"BETA"}"#,
    )?;
    show(&harness, "read", r#"{"path":"hello.txt"}"#)?;
    show(&harness, "bash", r#"{"command":"ls -1 && pwd"}"#)?;

    // serde_json's unknown-field and wrong-type paths, where the jump tables are.
    show(
        &harness,
        "read",
        r#"{"path":"hello.txt","bogus":{"a":[1,2,3]}}"#,
    )?;
    show(&harness, "read", r#"{"path":123}"#)?;

    // The boundary, from inside. Both must fail.
    show(&harness, "read", r#"{"path":"../../../etc/passwd"}"#)?;
    show(&harness, "read", r#"{"path":"/etc/passwd"}"#)?;
    show(
        &harness,
        "edit",
        r#"{"path":"hello.txt","old_string":"nope","new_string":"x"}"#,
    )?;

    Ok(())
}

fn show(harness: &Berm, tool: &str, args: &str) -> Result<()> {
    println!("$ {tool} {args}");
    match harness.call(tool, args.as_bytes().to_vec()) {
        Ok(Ok(result)) => println!("{result}\n"),
        Ok(Err(failure)) => println!("failed: {failure}\n"),
        Err(trapped) => println!("TRAPPED: {trapped:#}\n"),
    }
    Ok(())
}
