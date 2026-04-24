use crate::{client::OutlookClient, error::Error};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub start: DateTimeTimeZone,
    #[serde(default)]
    pub end: DateTimeTimeZone,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub is_all_day: bool,
    #[serde(default)]
    pub body_preview: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DateTimeTimeZone {
    #[serde(default)]
    pub date_time: String,
    #[serde(default)]
    pub time_zone: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(default)]
    pub display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateEventRequest {
    subject: String,
    start: DateTimeTimeZone,
    end: DateTimeTimeZone,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<crate::mail::Body>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateEventRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<DateTimeTimeZone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    end: Option<DateTimeTimeZone>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location: Option<Location>,
}

pub async fn list_events(client: &OutlookClient, start: &str, end: &str) -> Result<String, Error> {
    let resp = client
        .get(
            "/me/calendarView",
            &[
                ("startDateTime", start),
                ("endDateTime", end),
                ("$orderby", "start/dateTime"),
                (
                    "$select",
                    "id,subject,start,end,location,isAllDay,bodyPreview",
                ),
            ],
        )
        .await?;
    Ok(serde_json::to_string(&resp["value"])?)
}

pub async fn create_event(
    client: &OutlookClient,
    subject: &str,
    start: &str,
    end: &str,
    time_zone: &str,
    location: Option<&str>,
    body: Option<&str>,
) -> Result<String, Error> {
    let req = CreateEventRequest {
        subject: subject.to_owned(),
        start: DateTimeTimeZone {
            date_time: start.to_owned(),
            time_zone: time_zone.to_owned(),
        },
        end: DateTimeTimeZone {
            date_time: end.to_owned(),
            time_zone: time_zone.to_owned(),
        },
        location: location.map(|l| Location {
            display_name: l.to_owned(),
        }),
        body: body.map(|b| crate::mail::Body {
            content_type: "Text".to_owned(),
            content: b.to_owned(),
        }),
    };
    let resp = client.post("/me/events", &req).await?;
    Ok(serde_json::to_string(&resp)?)
}

pub async fn update_event(
    client: &OutlookClient,
    id: &str,
    subject: Option<&str>,
    start: Option<&str>,
    end: Option<&str>,
    time_zone: Option<&str>,
    location: Option<&str>,
) -> Result<String, Error> {
    let tz = time_zone.unwrap_or("UTC");
    let req = UpdateEventRequest {
        subject: subject.map(|s| s.to_owned()),
        start: start.map(|s| DateTimeTimeZone {
            date_time: s.to_owned(),
            time_zone: tz.to_owned(),
        }),
        end: end.map(|e| DateTimeTimeZone {
            date_time: e.to_owned(),
            time_zone: tz.to_owned(),
        }),
        location: location.map(|l| Location {
            display_name: l.to_owned(),
        }),
    };
    let resp = client.patch(&format!("/me/events/{id}"), &req).await?;
    Ok(serde_json::to_string(&resp)?)
}

pub async fn delete_event(client: &OutlookClient, id: &str) -> Result<String, Error> {
    client.delete(&format!("/me/events/{id}")).await?;
    Ok(r#"{"status":"deleted"}"#.to_owned())
}
