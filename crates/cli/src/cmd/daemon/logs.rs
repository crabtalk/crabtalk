//! `crabtalk daemon logs` — view daemon logs.

use anyhow::Result;

/// Display daemon log output by delegating to the shared `wcore::service::logs`.
pub fn logs(tail_args: &[String]) -> Result<()> {
    wcore::service::logs("daemon", tail_args)
}
