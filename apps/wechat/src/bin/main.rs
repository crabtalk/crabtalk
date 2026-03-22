//! `crabtalk-wechat` binary — WeChat gateway for Crabtalk.

use clap::Parser;
use crabtalk_wechat::{GatewayConfig, config::WechatConfig};

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

#[crabtalk_command::command(kind = "client", label = "ai.crabtalk.gateway-wechat")]
struct GatewayWechat;

impl GatewayWechat {
    async fn run(&self) -> anyhow::Result<()> {
        let config_path = wcore::paths::CONFIG_DIR.join("gateway.toml");
        let mut config = if config_path.exists() {
            GatewayConfig::load(&config_path)?
        } else {
            GatewayConfig::default()
        };

        // QR login on first run.
        if config.wechat.as_ref().is_none_or(|w| w.token.is_empty()) {
            let (token, base_url) = qr_login().await?;
            config.wechat = Some(WechatConfig {
                token,
                base_url,
                allowed_users: vec![],
            });
            config.save(&config_path)?;
            tracing::info!("saved config to {}", config_path.display());
        }

        let socket = wcore::paths::SOCKET_PATH.clone();
        crabtalk_wechat::serve::run(&socket.to_string_lossy(), &config).await
    }
}

#[derive(Parser)]
#[command(name = "crabtalk-wechat", about = "Crabtalk WeChat gateway")]
struct App {
    #[command(subcommand)]
    action: GatewayWechatCommand,
}

async fn qr_login() -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::new();
    let base_url = DEFAULT_BASE_URL;

    println!("Fetching QR code for WeChat login...");
    let qr = crabtalk_wechat::api::fetch_qrcode(&client, base_url).await?;
    println!("\nScan this QR code with WeChat:");
    println!("{}\n", qr.qrcode_img_content);
    println!("Waiting for scan...");

    let mut scanned = false;
    loop {
        let status = crabtalk_wechat::api::poll_qr_status(&client, base_url, &qr.qrcode).await?;
        match status.status.as_str() {
            "wait" => {}
            "scaned" => {
                if !scanned {
                    println!("Scanned! Confirm on your phone...");
                    scanned = true;
                }
            }
            "confirmed" => {
                let token = status
                    .bot_token
                    .ok_or_else(|| anyhow::anyhow!("confirmed but no bot_token"))?;
                let url = status.baseurl.unwrap_or_else(|| base_url.to_string());
                println!("Connected!");
                return Ok((token, url));
            }
            "expired" => {
                anyhow::bail!("QR code expired, please try again");
            }
            other => {
                anyhow::bail!("unexpected QR status: {other}");
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

fn main() {
    let app = App::parse();
    app.action.start(GatewayWechat);
}
