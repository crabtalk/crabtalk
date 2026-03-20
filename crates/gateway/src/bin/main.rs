//! `crabtalk-gateway` binary — manages gateway services.

use clap::{Parser, Subcommand};
use crabtalk_gateway::{GatewayConfig, config::TelegramConfig};
use dialoguer::{Password, theme::ColorfulTheme};
use wcore::service::{ClientService, Service};

struct TelegramService;

impl Service for TelegramService {
    fn name(&self) -> &str {
        "gateway-telegram"
    }
    fn description(&self) -> &str {
        "Crabtalk Gateway - Telegram"
    }
    fn label(&self) -> &str {
        "ai.crabtalk.gateway-telegram"
    }
    fn subcommand(&self) -> &str {
        "telegram"
    }
}

impl ClientService for TelegramService {
    async fn run(&self) -> anyhow::Result<()> {
        let socket = wcore::paths::SOCKET_PATH.clone();
        let config_path = wcore::paths::CONFIG_DIR.join("gateway.toml");
        let config = if config_path.exists() {
            GatewayConfig::load(&config_path)?
        } else {
            GatewayConfig::default()
        };
        crabtalk_gateway::telegram::serve::run(&socket.to_string_lossy(), &config).await
    }
}

#[derive(Parser)]
#[command(name = "crabtalk-gateway", about = "Crabtalk gateway manager")]
struct App {
    #[command(subcommand)]
    command: GatewayCommand,
}

#[derive(Subcommand)]
enum GatewayCommand {
    /// Manage the Telegram gateway.
    Telegram {
        #[command(subcommand)]
        action: TelegramAction,
    },
}

#[derive(Subcommand)]
enum TelegramAction {
    /// Install and start the Telegram gateway as a system service.
    Start,
    /// Stop and uninstall the Telegram gateway system service.
    Stop,
    /// Run the Telegram gateway directly (used by launchd/systemd).
    Run,
    /// View gateway service logs.
    Logs {
        /// Arguments passed through to `tail` (e.g. `-f`, `-n 100`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let level = std::env::var("RUST_LOG")
        .ok()
        .map(
            |v| match v.rsplit('=').next().unwrap_or(&v).to_lowercase().as_str() {
                "trace" => tracing::Level::TRACE,
                "debug" => tracing::Level::DEBUG,
                "info" => tracing::Level::INFO,
                "error" => tracing::Level::ERROR,
                _ => tracing::Level::WARN,
            },
        )
        .unwrap_or(tracing::Level::WARN);
    tracing_subscriber::fmt().with_max_level(level).init();

    let app = App::parse();
    match app.command {
        GatewayCommand::Telegram { action } => match action {
            TelegramAction::Start => {
                ensure_config()?;
                TelegramService.start()?;
            }
            TelegramAction::Stop => TelegramService.stop()?,
            TelegramAction::Run => TelegramService.run().await?,
            TelegramAction::Logs { tail_args } => TelegramService.logs(&tail_args)?,
        },
    }
    Ok(())
}
