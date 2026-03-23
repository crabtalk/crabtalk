use crate::{config::Config, error::Error, token::Token};
use serde::Deserialize;

const SCOPES: &str = "Mail.ReadWrite Mail.Send Calendars.ReadWrite offline_access";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

fn auth_url(config: &Config) -> String {
    format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize",
        config.tenant_id
    )
}

fn token_url(config: &Config) -> String {
    format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        config.tenant_id
    )
}

/// Start a local HTTP server, print the login URL, wait for the redirect,
/// exchange the auth code for tokens.
pub async fn authorize(config: &Config) -> Result<Token, Error> {
    let listener = std::net::TcpListener::bind(format!("127.0.0.1:{}", config.redirect_port))
        .map_err(|e| Error::Auth(format!("failed to bind port {}: {e}", config.redirect_port)))?;

    let redirect_uri = format!("http://localhost:{}", config.redirect_port);
    let url = format!(
        "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&response_mode=query",
        auth_url(config),
        config.client_id,
        urlencoding(&redirect_uri),
        urlencoding(SCOPES),
    );

    eprintln!("Open this URL to sign in:\n\n  {url}\n");
    eprintln!("Waiting for redirect...");

    let code = wait_for_code(&listener)?;
    exchange_code(config, &code, &redirect_uri).await
}

/// Accept one connection, parse the auth code from the query string, and send
/// a simple HTML response.
fn wait_for_code(listener: &std::net::TcpListener) -> Result<String, Error> {
    use std::io::{Read, Write};

    let (mut stream, _) = listener
        .accept()
        .map_err(|e| Error::Auth(format!("accept failed: {e}")))?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .map_err(|e| Error::Auth(format!("read failed: {e}")))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse "GET /?code=...&... HTTP/1.1"
    let code = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|path| path.split('?').nth(1))
        .and_then(|query| {
            query
                .split('&')
                .find(|p| p.starts_with("code="))
                .map(|p| p.trim_start_matches("code=").to_owned())
        })
        .ok_or_else(|| Error::Auth("no auth code in redirect".into()))?;

    let body =
        "<!DOCTYPE html><html><body><h2>Authenticated! You can close this tab.</h2></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());

    Ok(code)
}

/// Encode form params as application/x-www-form-urlencoded body.
fn form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoding(k), urlencoding(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// POST a form-encoded request to the token endpoint.
async fn token_request(url: &str, params: &[(&str, &str)]) -> Result<Token, Error> {
    let client = reqwest::Client::new();
    let resp: TokenResponse = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form_body(params))
        .send()
        .await?
        .error_for_status()
        .map_err(|e| Error::Auth(format!("token request failed: {e}")))?
        .json()
        .await?;

    Ok(Token {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at: chrono::Utc::now().timestamp() + resp.expires_in,
    })
}

/// Exchange the authorization code for access + refresh tokens.
async fn exchange_code(config: &Config, code: &str, redirect_uri: &str) -> Result<Token, Error> {
    token_request(
        &token_url(config),
        &[
            ("client_id", config.client_id.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("scope", SCOPES),
        ],
    )
    .await
}

/// Refresh an expired access token using the refresh token.
pub async fn refresh(config: &Config, refresh_token: &str) -> Result<Token, Error> {
    token_request(
        &token_url(config),
        &[
            ("client_id", config.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
            ("scope", SCOPES),
        ],
    )
    .await
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}
