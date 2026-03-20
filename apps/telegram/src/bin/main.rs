//! `crabtalk-telegram` binary — Telegram gateway for Crabtalk.

use clap::Parser;
use crabtalk_telegram::{GatewayConfig, config::TelegramConfig};
use dialoguer::{Password, theme::ColorfulTheme};

#[crabtalk_command::command(kind = "client", label = "ai.crabtalk.gateway-telegram")]
struct GatewayTelegram;

impl GatewayTelegram {
    async fn run(&self) -> anyhow::Result<()> {
        let socket = wcore::paths::SOCKET_PATH.clone();
        let config_path = wcore::paths::CONFIG_DIR.join("gateway.toml");
        let config = if config_path.exists() {
            GatewayConfig::load(&config_path)?
        } else {
            GatewayConfig::default()
        };
        crabtalk_telegram::serve::run(&socket.to_string_lossy(), &config).await
    }
}

#[derive(Parser)]
#[command(name = "crabtalk-telegram", about = "Crabtalk Telegram gateway")]
struct App {
    #[command(subcommand)]
    action: GatewayTelegramCommand,
}

fn default_config_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR.join("gateway.toml")
}

fn ensure_config() -> anyhow::Result<()> {
    let config_path = default_config_path();
    let mut config = if config_path.exists() {
        GatewayConfig::load(&config_path)?
    } else {
        GatewayConfig::default()
    };

    if config.telegram.as_ref().is_none_or(|t| t.token.is_empty()) {
        let token = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Telegram bot token (from @BotFather)")
            .interact()?;
        if token.is_empty() {
            anyhow::bail!("token cannot be empty");
        }
        config.telegram = Some(TelegramConfig {
            token,
            allowed_users: vec![],
        });
        config.save(&config_path)?;
        println!("saved config to {}", config_path.display());
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
