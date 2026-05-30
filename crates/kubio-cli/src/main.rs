mod args;
mod commands;
mod config;

use anyhow::Result;
use args::{Cli, Command};
use clap::Parser;
use kubio_telemetry::init_tracing;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => commands::serve(args).await,
        Command::Routes(args) => commands::routes(args).await,
        Command::Explain(args) => commands::explain(args).await,
        Command::Doctor(args) => commands::doctor(args).await,
        Command::Purge(args) => commands::purge(args).await,
        Command::Update(args) => commands::update(args).await,
    }
}
