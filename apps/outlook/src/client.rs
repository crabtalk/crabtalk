use crate::{
    auth,
    config::Config,
    error::Error,
    token::{Token, token_path},
};
use serde::Serialize;
use tokio::sync::RwLock;

const BASE_URL: &str = "https://graph.microsoft.com/v1.0";

pub struct OutlookClient {
    http: reqwest::Client,
    token: RwLock<Token>,
    config: Config,
}

impl OutlookClient {
    pub fn new(config: Config) -> Result<Self, Error> {
        let token = Token::load(&token_path()).map_err(|e| {
            Error::Auth(format!(
                "no token found — run `crabtalk-outlook auth` first: {e}"
            ))
        })?;
        Ok(Self {
            http: reqwest::Client::new(),
            token: RwLock::new(token),
            config,
        })
    }

    async fn ensure_token(&self) -> Result<String, Error> {
        {
            let tok = self.token.read().await;
            if !tok.is_expired() {
                return Ok(tok.access_token.clone());
            }
        }

        let mut tok = self.token.write().await;
        // Double-check after acquiring write lock
        if !tok.is_expired() {
            return Ok(tok.access_token.clone());
        }

        let refreshed = auth::refresh(&self.config, &tok.refresh_token).await?;
        refreshed.save(&token_path())?;
        *tok = refreshed;
        Ok(tok.access_token.clone())
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<serde_json::Value, Error> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .get(format!("{BASE_URL}{path}"))
            .bearer_auth(&token)
            .query(query)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::Api(e.to_string()))?
            .json()
            .await?;
        Ok(resp)
    }

    pub async fn post(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<serde_json::Value, Error> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .post(format!("{BASE_URL}{path}"))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::Api(e.to_string()))?;

        let text = resp.text().await?;
        if text.is_empty() {
            Ok(serde_json::Value::Null)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }

    pub async fn patch(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<serde_json::Value, Error> {
        let token = self.ensure_token().await?;
        let resp = self
            .http
            .patch(format!("{BASE_URL}{path}"))
            .bearer_auth(&token)
            .json(body)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::Api(e.to_string()))?;

        let text = resp.text().await?;
        if text.is_empty() {
            Ok(serde_json::Value::Null)
        } else {
            Ok(serde_json::from_str(&text)?)
        }
    }

    pub async fn delete(&self, path: &str) -> Result<(), Error> {
        let token = self.ensure_token().await?;
        self.http
            .delete(format!("{BASE_URL}{path}"))
            .bearer_auth(&token)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| Error::Api(e.to_string()))?;
        Ok(())
    }
}
