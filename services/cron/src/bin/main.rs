//! `crabtalk-cron` — cron scheduler service.
//!
//! Admin is direct edits to `$CRABTALK_HOME/config/crons.toml`. The running
//! scheduler polls the file's mtime and reconciles timers on change.

use clap::Parser;

#[command::command(kind = "client", name = "cron")]
struct CronService;

impl CronService {
    async fn run(&self) -> anyhow::Result<()> {
        let client = sdk::NodeClient::platform_default()?;
        crabtalk_cron::run(schedule_path(), client).await
    }
}

fn schedule_path() -> std::path::PathBuf {
    wcore::paths::CONFIG_DIR.join("config").join("crons.toml")
}

fn main() {
    CrabtalkCli::parse().start(CronService);
}
