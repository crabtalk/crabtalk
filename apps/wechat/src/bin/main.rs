//! `crabtalk-wechat` binary — WeChat gateway for Crabtalk.

use clap::Parser;
use crabtalk_wechat::config::WechatConfig;

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";

#[crabtalk_command::command(kind = "client", name = "wechat")]
struct GatewayWechat;

impl GatewayWechat {
    async fn run(&self) -> anyhow::Result<()> {
        let client = gateway::NodeClient::platform_default()?;
        let path = config_path();
        let config = WechatConfig::load(&path)?;
        crabtalk_wechat::serve::run(client, &config).await
    }
}

fn config_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR.join("config").join("wechat.toml")
}

/// Ensure a WeChat token exists in config/wechat.toml, running QR login if needed.
///
/// Runs before the service runtime starts (same pattern as Telegram's
/// ensure_config). Uses a one-off tokio runtime for the HTTP calls.
fn ensure_config() -> anyhow::Result<()> {
    let path = config_path();
    let needs_token = if path.exists() {
        WechatConfig::load(&path)
            .map(|c| c.token.is_empty())
            .unwrap_or(true)
    } else {
        true
    };

    if needs_token {
        let rt = tokio::runtime::Runtime::new()?;
        let (token, base_url) = rt.block_on(qr_login())?;
        let config = WechatConfig {
            token,
            base_url,
            allowed_users: vec![],
        };
        config.save(&path)?;
        println!("saved config to {}", path.display());
    }
    Ok(())
}

async fn qr_login() -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::new();
    let base_url = DEFAULT_BASE_URL;

    println!("Fetching QR code for WeChat login...");
    let qr = crabtalk_wechat::api::fetch_qrcode(&client, base_url).await?;
    println!("\nScan this QR code with WeChat:\n");
    qr2term::print_qr(&qr.qrcode_img_content)?;
    println!();
    println!("Waiting for scan...");

    let mut scanned = false;
    loop {
        let status = crabtalk_wechat::api::poll_qr_status(&client, base_url, &qr.qrcode).await?;
        match status.status.as_str() {
            "wait" => {}
            // NOTE: the server returns "scaned" (sic) — not a typo on our side.
            "scaned" | "scanned" => {
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
    // Migrate: remove old gateway-prefixed service if present.
    if crabtalk_command::is_installed("ai.crabtalk.gateway-wechat") {
        let _ = crabtalk_command::uninstall("ai.crabtalk.gateway-wechat");
    }

    let cli = CrabtalkCli::parse();
    if matches!(&cli.action, GatewayWechatCommand::Start { .. })
        && let Err(e) = ensure_config()
    {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
    cli.start(GatewayWechat);
}
