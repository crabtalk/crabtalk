//! Reaching the daemon, and saying so when it isn't there.

use anyhow::{Context, Result, bail};
use client::{ConnectionInfo, Transport};
use std::io::ErrorKind;

/// Connect to the daemon under `$CRABTALK_HOME`.
///
/// Nothing starts the daemon on a client's behalf — there is no service
/// manager and no auto-spawn — so the absence of one is a sentence about
/// what to run. Any other failure keeps its cause, which is the difference
/// between "not started" and "started, and you cannot open its socket".
pub async fn connect() -> Result<Transport> {
    let info = ConnectionInfo::platform_default()?;
    let error = match client::connect_from(&info).await {
        Ok(transport) => return Ok(transport),
        Err(error) => error,
    };

    let kind = error.downcast_ref::<std::io::Error>().map(|e| e.kind());
    if matches!(
        kind,
        Some(ErrorKind::NotFound | ErrorKind::ConnectionRefused)
    ) {
        bail!("cannot reach the crabtalk daemon at {info} — start it with `crabtalkd`");
    }
    Err(error).with_context(|| format!("cannot reach the crabtalk daemon at {info}"))
}
