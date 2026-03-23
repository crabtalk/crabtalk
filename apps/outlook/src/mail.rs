use crate::{client::OutlookClient, error::Error};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub from: Option<Recipient>,
    #[serde(default)]
    pub to_recipients: Vec<Recipient>,
    #[serde(default)]
    pub received_date_time: String,
    #[serde(default)]
    pub body_preview: String,
    #[serde(default)]
    pub is_read: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDetail {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub from: Option<Recipient>,
    #[serde(default)]
    pub to_recipients: Vec<Recipient>,
    #[serde(default)]
    pub received_date_time: String,
    #[serde(default)]
    pub body: Body,
    #[serde(default)]
    pub is_read: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipient {
    pub email_address: EmailAddress,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub address: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SendMailRequest {
    message: OutgoingMessage,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutgoingMessage {
    subject: String,
    body: Body,
    to_recipients: Vec<Recipient>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplyRequest {
    comment: String,
}

pub async fn list_mail(client: &OutlookClient, count: u32) -> Result<String, Error> {
    let top = count.to_string();
    let resp = client
        .get(
            "/me/messages",
            &[
                ("$top", top.as_str()),
                ("$orderby", "receivedDateTime desc"),
                (
                    "$select",
                    "id,subject,from,toRecipients,receivedDateTime,bodyPreview,isRead",
                ),
            ],
        )
        .await?;
    Ok(serde_json::to_string(&resp["value"])?)
}

pub async fn read_mail(client: &OutlookClient, id: &str) -> Result<String, Error> {
    let resp = client.get(&format!("/me/messages/{id}"), &[]).await?;
    let detail: MessageDetail = serde_json::from_value(resp)?;
    Ok(serde_json::to_string(&detail)?)
}

pub async fn search_mail(client: &OutlookClient, query: &str) -> Result<String, Error> {
    let resp = client
        .get(
            "/me/messages",
            &[
                ("$search", &format!("\"{query}\"")),
                ("$top", "25"),
                (
                    "$select",
                    "id,subject,from,toRecipients,receivedDateTime,bodyPreview,isRead",
                ),
            ],
        )
        .await?;
    Ok(serde_json::to_string(&resp["value"])?)
}

pub async fn send_mail(
    client: &OutlookClient,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<String, Error> {
    let req = SendMailRequest {
        message: OutgoingMessage {
            subject: subject.to_owned(),
            body: Body {
                content_type: "Text".to_owned(),
                content: body.to_owned(),
            },
            to_recipients: vec![Recipient {
                email_address: EmailAddress {
                    name: String::new(),
                    address: to.to_owned(),
                },
            }],
        },
    };
    client.post("/me/sendMail", &req).await?;
    Ok(r#"{"status":"sent"}"#.to_owned())
}

pub async fn reply_mail(client: &OutlookClient, id: &str, comment: &str) -> Result<String, Error> {
    let req = ReplyRequest {
        comment: comment.to_owned(),
    };
    client
        .post(&format!("/me/messages/{id}/reply"), &req)
        .await?;
    Ok(r#"{"status":"replied"}"#.to_owned())
}
