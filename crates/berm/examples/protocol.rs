//! Exercise the protocol door: the grant, the allowlist, and the redaction.
//!
//! ```sh
//! cargo build --release -p berm-peers --target riscv64imac-unknown-none-elf
//! cargo run --example protocol -p berm
//! ```
//!
//! The runtime here is a stand-in, because none of what this checks is about
//! the runtime: whether an ungranted harness can reach the door at all,
//! whether an ungranted *message* gets past the allowlist, and whether
//! `AgentInfo.config` survives the trip. A real daemon would answer the same
//! `ClientMessage` the same way.

use anyhow::{Context, Result};
use berm::{Berm, Config, Engine};
use crabtalk_berm::Dispatch;

use proto::{AgentInfo, AgentList, ServerMessage};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

const HARNESS: &str = "target/riscv64imac-unknown-none-elf/release/peers";

// Mirrors the real dispatch path: a harness blocks the thread it runs on, so
// the hook hands invocations to the blocking pool and system harnesses `block_on`
// from inside one. Calling from an async context instead would panic.
#[tokio::main]
async fn main() -> Result<()> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .context("no workspace root")?
        .to_path_buf();
    let elf = fs::read(workspace.join(HARNESS)).with_context(|| {
        format!("build the harness first: cargo build --release -p berm-peers --target riscv64imac-unknown-none-elf ({HARNESS})")
    })?;

    let engine = Engine::new(&Config::new())?;

    // A stand-in runtime that always answers with one agent, whose `config`
    // carries something a harness must never see.
    let protocol: Arc<OnceLock<Dispatch>> = Arc::new(OnceLock::new());
    let dispatch: Dispatch = Arc::new(|_msg| {
        Box::pin(async {
            vec![ServerMessage {
                msg: Some(proto::server_message::Msg::AgentList(AgentList {
                    agents: vec![AgentInfo {
                        name: "reviewer".into(),
                        description: "reads diffs".into(),
                        config: r#"{"mcps":[{"auth":"Bearer SECRET"}]}"#.into(),
                        ..Default::default()
                    }],
                })),
            }]
        })
    });
    let _ = protocol.set(dispatch);

    let granted = Berm::load(
        &engine,
        &elf,
        &[crabtalk_berm::Protocol::new(
            protocol.clone(),
            tokio::runtime::Handle::current(),
            crabtalk_berm::Scope {
                read: true,
                sessions: false,
                skills: Vec::new(),
                agent: store::AgentId::default(),
            },
        )
        .harness()],
    )?;
    let ungranted = Berm::load(&engine, &elf, &[])?;

    tokio::task::spawn_blocking(move || {
        println!("== granted protocol:read ==");
        show(&granted);
        println!("== not granted ==");
        show(&ungranted);
    })
    .await?;

    Ok(())
}

fn show(harness: &Berm) {
    match harness.call("peers", Vec::new()) {
        Ok(Ok(result)) => println!("{result}"),
        Ok(Err(failure)) => println!("failed: {failure}\n"),
        Err(trapped) => println!("TRAPPED: {trapped:#}\n"),
    }
}
