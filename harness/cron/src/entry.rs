//! Cron entry data type and validators.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// A single scheduled trigger.
///
/// Fires `/{skill}` into `agent` as `sender` on the cron `schedule`, skipping
/// fire times that fall inside the optional quiet window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CronEntry {
    pub id: u64,
    pub schedule: String,
    pub skill: String,
    pub agent: String,
    pub sender: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_end: Option<String>,
    #[serde(default)]
    pub once: bool,
}

/// Parse a cron expression, returning a human-readable error on failure.
pub fn validate_schedule(schedule: &str) -> Result<(), String> {
    ::cron::Schedule::from_str(schedule)
        .map(|_| ())
        .map_err(|e| format!("invalid cron schedule '{schedule}': {e}"))
}

/// Is now inside the quiet window? Requires both `start` and `end` to be set
/// and parseable as `%H:%M`; otherwise returns false (no suppression).
///
/// Windows wrapping midnight are supported: `start="23:00"`, `end="07:00"`
/// suppresses from 23:00 through 06:59.
pub fn is_quiet(start: Option<&str>, end: Option<&str>) -> bool {
    let (Some(qs), Some(qe)) = (start, end) else {
        return false;
    };
    let Ok(qs) = chrono::NaiveTime::parse_from_str(qs, "%H:%M") else {
        return false;
    };
    let Ok(qe) = chrono::NaiveTime::parse_from_str(qe, "%H:%M") else {
        return false;
    };
    let now = chrono::Local::now().time();
    if qs <= qe {
        now >= qs && now < qe
    } else {
        now >= qs || now < qe
    }
}
