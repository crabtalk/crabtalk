//! The crabtalk daemon.

use anyhow::{Result, bail};

#[tokio::main]
async fn main() -> Result<()> {
    parse()?;

    let level = std::env::var("RUST_LOG")
        .map(|val| parse_level(&val))
        .unwrap_or(tracing::Level::INFO);
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(level)
        .init();

    crabtalk_agent::daemon::start().await
}

fn usage() -> String {
    let home = crabup::dirs::HOME_VAR;
    format!(
        "\
crabtalk-agent — the crabtalk daemon

USAGE:
    crabtalk-agent [OPTIONS]

OPTIONS:
    -h, --help     Print this message
    -V, --version  Print the version

ENVIRONMENT:
    {home}  Install root (default: ~/.crabtalk)

It listens on ${home}/run/crabtalk.sock and on a TCP port recorded in
${home}/run/crabtalk.port, and runs in the foreground until SIGTERM.
"
    )
}

/// The daemon takes no arguments, so the only ones it accepts print and
/// exit. Anything else, including a second one, is a usage error.
fn parse() -> Result<()> {
    let Some(arg) = std::env::args().nth(1) else {
        return Ok(());
    };
    match arg.as_str() {
        "-h" | "--help" => {
            print!("{}", usage());
            std::process::exit(0);
        }
        "-V" | "--version" => {
            println!("crabtalk-agent {}", env!("CARGO_PKG_VERSION"));
            std::process::exit(0);
        }
        other => bail!("unexpected argument: {other}\n\n{}", usage()),
    }
}

/// Extract the most specific level from a filter string like "crabtalk=debug".
fn parse_level(s: &str) -> tracing::Level {
    match s.rsplit('=').next().unwrap_or(s).to_lowercase().as_str() {
        "trace" => tracing::Level::TRACE,
        "debug" => tracing::Level::DEBUG,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    }
}
