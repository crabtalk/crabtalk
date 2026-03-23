//! `crabtalk-telegram` binary — Telegram gateway for Crabtalk.

use clap::Parser;
use crabtalk_telegram::config::TelegramConfig;
use dialoguer::{Password, theme::ColorfulTheme};

#[crabtalk_command::command(kind = "client", label = "ai.crabtalk.gateway-telegram")]
struct GatewayTelegram;

impl GatewayTelegram {
    async fn run(&self) -> anyhow::Result<()> {
        let socket = wcore::paths::SOCKET_PATH.clone();
        let config_path = config_path();
        let config = TelegramConfig::load(&config_path)?;
        crabtalk_telegram::serve::run(&socket.to_string_lossy(), &config).await
    }
}

#[derive(Parser)]
#[command(name = "crabtalk-telegram", about = "Crabtalk Telegram gateway")]
struct App {
    #[command(subcommand)]
    action: GatewayTelegramCommand,
}

fn config_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR
        .join("config")
        .join("telegram.toml")
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
        let token = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Telegram bot token (from @BotFather)")
            .interact()?;
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
    let app = App::parse();
    if matches!(&app.action, GatewayTelegramCommand::Start { .. })
        && let Err(e) = ensure_config()
    {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    app.action.start(GatewayTelegram);
}
