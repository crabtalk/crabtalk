//! `crabtalk-telegram` binary — Telegram gateway for Crabtalk.

use std::io::Write;

use clap::Parser;
use crabtalk_telegram::config::TelegramConfig;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};

#[crabtalk_command::command(kind = "client", name = "telegram")]
struct GatewayTelegram;

impl GatewayTelegram {
    async fn run(&self) -> anyhow::Result<()> {
        let client = gateway::NodeClient::platform_default()?;
        let config_path = config_path();
        let config = TelegramConfig::load(&config_path)?;
        crabtalk_telegram::serve::run(client, &config).await
    }
}

fn config_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR
        .join("config")
        .join("telegram.toml")
}

fn read_masked(prompt: &str) -> anyhow::Result<String> {
    let mut stderr = std::io::stderr();
    write!(stderr, "\x1b[32m?\x1b[0m \x1b[1m{prompt}\x1b[0m: ")?;
    stderr.flush()?;

    enable_raw_mode()?;
    let result = read_masked_raw(&mut stderr);
    disable_raw_mode()?;
    writeln!(stderr)?;
    stderr.flush()?;

    result
}

fn read_masked_raw(w: &mut impl Write) -> anyhow::Result<String> {
    let mut input = String::new();
    loop {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Enter => return Ok(input),
                KeyCode::Backspace if !input.is_empty() => {
                    input.pop();
                    write!(w, "\x08 \x08")?;
                    w.flush()?;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    anyhow::bail!("interrupted");
                }
                KeyCode::Char(c) if !c.is_control() => {
                    input.push(c);
                    write!(w, "*")?;
                    w.flush()?;
                }
                _ => {}
            }
        }
    }
}

fn ensure_config() -> anyhow::Result<()> {
    let path = config_path();
    let needs_token = if path.exists() {
        TelegramConfig::load(&path)
            .map(|c| c.token.is_empty())
            .unwrap_or(true)
    } else {
        true
    };

    if needs_token {
        let token = read_masked("Telegram bot token (from @BotFather)")?;
        if token.is_empty() {
            anyhow::bail!("token cannot be empty");
        }
        let config = TelegramConfig {
            token,
            allowed_users: vec![],
        };
        config.save(&path)?;
        println!("saved config to {}", path.display());
    }
    Ok(())
}

fn main() {
    // Migrate: remove old gateway-prefixed service if present.
    if crabtalk_command::is_installed("ai.crabtalk.gateway-telegram") {
        let _ = crabtalk_command::uninstall("ai.crabtalk.gateway-telegram");
    }

    let cli = CrabtalkCli::parse();
    if matches!(&cli.action, GatewayTelegramCommand::Start { .. })
        && let Err(e) = ensure_config()
    {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    cli.start(GatewayTelegram);
}
