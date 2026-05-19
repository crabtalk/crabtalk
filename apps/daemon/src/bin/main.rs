//! Crabtalk daemon binary entry point.

use clap::Parser;
use crabtalkd::{CrabtalkCli, Daemon};

fn main() {
    CrabtalkCli::parse().start(Daemon);
}
