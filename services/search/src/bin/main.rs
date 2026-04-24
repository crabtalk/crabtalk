use clap::Parser;
use crabtalk_search::cmd::{CrabtalkCli, Mcp};

fn main() {
    let cli = CrabtalkCli::parse();
    cli.start(Mcp);
}
