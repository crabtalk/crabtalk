//! `crabtalk-cron` — cron scheduler binary.
//!
//! Three roles:
//! - `run`: scheduler daemon (invoked by launchd/systemd).
//! - `start`/`stop`/`logs`: install/uninstall/tail the system service.
//! - `create`/`list`/`delete`: admin the schedule file.

use anyhow::Result;
use clap::{Parser, Subcommand};
use command::Service;
use crabtalk_cron::Store;
use sdk::NodeClient;
use std::path::PathBuf;
use wcore::trigger::cron::CronEntry;

const NAME: &str = "cron";
const LABEL: &str = "ai.crabtalk.cron";
const DESCRIPTION: &str = "Cron scheduler for Crabtalk";

struct CronService;

impl Service for CronService {
    fn name(&self) -> &str {
        NAME
    }
    fn description(&self) -> &str {
        DESCRIPTION
    }
    fn label(&self) -> &str {
        LABEL
    }
}

#[derive(Debug, Parser)]
#[command(name = "crabtalk-cron")]
struct Cli {
    /// Increase log verbosity (-v = info, -vv = debug, -vvv = trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// Install and start the cron service.
    Start {
        /// Re-install even if already installed.
        #[arg(short, long)]
        force: bool,
    },
    /// Stop and uninstall the cron service.
    Stop,
    /// Run the scheduler directly (used by launchd/systemd).
    Run,
    /// View cron service logs.
    Logs {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        tail_args: Vec<String>,
    },
    /// Create a new schedule.
    Create {
        /// Cron expression — e.g. "0 */2 * * * *".
        #[arg(long)]
        schedule: String,
        /// Skill to invoke when the schedule fires.
        #[arg(long)]
        skill: String,
        /// Agent that owns the conversation.
        #[arg(long)]
        agent: String,
        /// Sender id (default: "cron").
        #[arg(long, default_value = "cron")]
        sender: String,
        /// Start of the quiet window (HH:MM, local time).
        #[arg(long)]
        quiet_start: Option<String>,
        /// End of the quiet window (HH:MM, local time).
        #[arg(long)]
        quiet_end: Option<String>,
        /// Fire once, then delete.
        #[arg(long)]
        once: bool,
    },
    /// List all schedules.
    List,
    /// Delete a schedule by id.
    Delete { id: u64 },
}

fn main() {
    let cli = Cli::parse();
    command::run(cli.verbose, move || async move { exec(cli.action).await });
}

async fn exec(action: Action) -> Result<()> {
    match action {
        Action::Start { force } => CronService.start(force)?,
        Action::Stop => CronService.stop()?,
        Action::Logs { tail_args } => CronService.logs(&tail_args)?,
        Action::Run => {
            let client = NodeClient::platform_default()?;
            crabtalk_cron::run(schedule_path(), client).await?;
        }
        Action::Create {
            schedule,
            skill,
            agent,
            sender,
            quiet_start,
            quiet_end,
            once,
        } => {
            let mut store = Store::load(schedule_path())?;
            let entry = store.create(CronEntry {
                id: 0,
                schedule,
                skill,
                agent,
                sender,
                quiet_start,
                quiet_end,
                once,
            })?;
            println!("created cron {}", entry.id);
        }
        Action::List => {
            let store = Store::load(schedule_path())?;
            if store.list().is_empty() {
                println!("no schedules at {}", store.path().display());
                return Ok(());
            }
            for e in store.list() {
                print!(
                    "{:>3}  {:<20}  /{:<16}  agent={} sender={}",
                    e.id, e.schedule, e.skill, e.agent, e.sender,
                );
                if let (Some(qs), Some(qe)) = (&e.quiet_start, &e.quiet_end) {
                    print!(" quiet={qs}..{qe}");
                }
                if e.once {
                    print!(" once");
                }
                println!();
            }
        }
        Action::Delete { id } => {
            let mut store = Store::load(schedule_path())?;
            if store.delete(id)? {
                println!("deleted cron {id}");
            } else {
                anyhow::bail!("cron {id} not found");
            }
        }
    }
    Ok(())
}

fn schedule_path() -> PathBuf {
    wcore::paths::CONFIG_DIR.join("cron").join("crons.toml")
}
