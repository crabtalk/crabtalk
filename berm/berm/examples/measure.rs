//! What a harness invocation costs.
//!
//! RFC 0205 puts store-per-invocation on the critical path for every tool
//! call, so this measures the parts that decide whether that holds: compiling
//! an ELF cold and warm, and one full invocation — instantiate, argument
//! transfer, guest call, result read, teardown.
//!
//! ```sh
//! cargo build --release -p berm-fixture --target riscv64imac-unknown-none-elf
//! cargo run --release --example measure -p berm
//! ```

use anyhow::{Context, Result};
use berm::Berm;
use rvtime::{Config, Engine};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const GUEST: &str = "target/riscv64imac-unknown-none-elf/release/fixture";
const ROUNDS: usize = 1000;

fn main() -> Result<()> {
    // Only the guest's own log; cranelift is chatty at info.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,harness=info"))
        .init();

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .context("no workspace root")?
        .to_path_buf();

    let elf = fs::read(root.join(GUEST)).with_context(|| {
        format!("build the guest first: cargo build --release -p berm-fixture --target riscv64imac-unknown-none-elf ({GUEST})")
    })?;
    println!("guest: {} bytes", elf.len());

    let cache = std::env::temp_dir().join("berm-measure");
    let _ = fs::remove_dir_all(&cache);

    let cold = time(|| compile(&cache, &elf))?;
    println!("compile (cold cache):  {cold:>10.3?}");

    let warm = time(|| compile(&cache, &elf))?;
    println!("compile (warm cache):  {warm:>10.3?}");

    let harness = compile(&cache, &elf)?;
    println!("manifest:              {:?}", harness.manifest());

    println!(
        "heap probe:            {:?}",
        harness.call("probe", b"".to_vec())?
    );

    // A payload in the range a real tool call carries.
    let args = format!(r#"{{"query":"{}"}}"#, "x".repeat(256));
    let echoed = harness
        .call("echo", args.as_bytes())?
        .map_err(anyhow::Error::msg)?;
    assert!(echoed.contains(&args), "round trip lost the payload");
    println!(
        "failure path:          {:?}",
        harness.call("boom", b"".to_vec())?.unwrap_err()
    );

    let mut chatty = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let start = Instant::now();
        let _ = harness.call("chatty", b"".to_vec())?;
        chatty.push(start.elapsed());
    }
    chatty.sort();
    println!("  +100 host calls:     {:>10.3?}", chatty[ROUNDS / 2]);

    let mut samples = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let start = Instant::now();
        let _ = harness.call("echo", args.as_bytes())?;
        samples.push(start.elapsed());
    }
    samples.sort();

    println!("invocations:           {ROUNDS}");
    println!("  min:                 {:>10.3?}", samples[0]);
    println!("  p50:                 {:>10.3?}", samples[ROUNDS / 2]);
    println!(
        "  p99:                 {:>10.3?}",
        samples[ROUNDS * 99 / 100]
    );
    println!("  max:                 {:>10.3?}", samples[ROUNDS - 1]);
    println!(
        "  mean:                {:>10.3?}",
        samples.iter().sum::<Duration>() / ROUNDS as u32
    );

    // Instantiate maps a guest address space per invocation. If that cost
    // tracked the configured size, a harness wanting room would pay for it on
    // every call.
    println!("p50 by guest memory size:");
    for mib in [16u64, 64, 256, 1024] {
        let mut config = Config::new();
        config.cache_dir(&cache).memory_size(mib * 1024 * 1024);
        let engine = Engine::new(&config)?;
        let harness = Berm::load(&engine, &elf, &[])?;

        let mut samples = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            let start = Instant::now();
            let _ = harness.call("echo", args.as_bytes())?;
            samples.push(start.elapsed());
        }
        samples.sort();
        println!("  {mib:>5} MiB:          {:>10.3?}", samples[ROUNDS / 2]);
    }

    Ok(())
}

fn compile(cache: &std::path::Path, elf: &[u8]) -> Result<Berm> {
    let mut config = Config::new();
    config.cache_dir(cache);
    let engine = Engine::new(&config)?;
    Berm::load(&engine, elf, &[])
}

fn time<T>(f: impl FnOnce() -> Result<T>) -> Result<Duration> {
    let start = Instant::now();
    f()?;
    Ok(start.elapsed())
}
