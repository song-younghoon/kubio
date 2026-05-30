mod args;
mod client;
#[cfg(feature = "experimental-http3")]
mod h3;
mod origin;
mod proxy;
mod report;
mod runner;

use anyhow::{bail, Result};
use args::{Args, OutputFormat};
use clap::Parser;
use report::print_text_report;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let report = runner::run(args.protocol, args.scenario, args.requests).await?;
    match args.output {
        OutputFormat::Text => print_text_report(&report),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if args.fail_on_budget && !report.budget.passed {
        bail!("benchmark budget failed");
    }
    Ok(())
}
