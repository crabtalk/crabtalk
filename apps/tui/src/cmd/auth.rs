//! Cloud authentication commands.

use anyhow::{Result, bail};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CLOUD_URL: &str = "http://localhost:5252";

pub async fn login() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let url = format!("{CLOUD_URL}/auth/google?client=terminal&port={port}&scope=llm");

    println!("Opening browser for login...");
    open_browser(&url)?;
    println!("Waiting for authentication (listening on port {port})...");

    let (mut stream, _) = listener.accept().await?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let token = parse_token(&request).ok_or_else(|| {
        anyhow::anyhow!("callback did not contain a token — login may have failed")
    })?;

    let response = "HTTP/1.1 200 OK\r\n\
        Content-Type: text/html\r\n\
        Connection: close\r\n\r\n\
        <html><body><h3>Logged in! You can close this tab.</h3></body></html>";
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    drop(stream);
    drop(listener);

    write_config(&token)?;
    println!(
        "Logged in — gateway key written to {}",
        config_path().display()
    );
    Ok(())
}

pub fn logout() -> Result<()> {
    let path = config_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Value =
        toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));

    if let Some(table) = doc.as_table_mut() {
        table.remove("llm");
    }

    let out = toml::to_string_pretty(&doc)?;
    std::fs::write(&path, out)?;
    println!("Logged out — removed [llm] from {}", path.display());
    Ok(())
}

fn parse_token(request: &str) -> Option<String> {
    let line = request.lines().next()?;
    let path = line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            return Some(value.to_owned());
        }
    }
    None
}

fn write_config(token: &str) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Value =
        toml::from_str(&raw).unwrap_or(toml::Value::Table(Default::default()));

    let table = doc.as_table_mut().unwrap();
    let llm = table
        .entry("llm")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    let llm_table = llm.as_table_mut().unwrap();
    llm_table.insert(
        "base_url".to_owned(),
        toml::Value::String(format!("{CLOUD_URL}/v1")),
    );
    llm_table.insert("api_key".to_owned(), toml::Value::String(token.to_owned()));

    let out = toml::to_string_pretty(&doc)?;
    std::fs::write(&path, out)?;
    Ok(())
}

fn config_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR.join(wcore::paths::CONFIG_FILE)
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("open").arg(url).status();
    #[cfg(target_os = "linux")]
    let cmd = std::process::Command::new("xdg-open").arg(url).status();
    #[cfg(target_os = "windows")]
    let cmd = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .status();

    match cmd {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => bail!("browser exited with {s}"),
        Err(e) => bail!("failed to open browser: {e}"),
    }
}
