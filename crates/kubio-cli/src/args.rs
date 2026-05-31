use clap::{Args, Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "kubio", version, about = "Safe API response reuse autopilot")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Serve(ServeArgs),
    Routes(AdminArgs),
    Explain(ExplainArgs),
    Doctor(DoctorArgs),
    Purge(PurgeArgs),
    Config(ConfigArgs),
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    #[arg(long = "to")]
    pub(crate) origin: Option<String>,
    #[arg(long, help = "proxy listen address; default: 0.0.0.0:8080")]
    pub(crate) listen: Option<SocketAddr>,
    #[arg(long, help = "dashboard listen address; default: 127.0.0.1:9900")]
    pub(crate) dashboard: Option<SocketAddr>,
    #[arg(long, help = "runtime mode: watch, shadow, or auto; default: watch")]
    pub(crate) mode: Option<String>,
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    #[arg(
        long,
        help = "freshness profile: strict, balanced, relaxed; default: balanced"
    )]
    pub(crate) freshness: Option<String>,
    #[arg(long)]
    pub(crate) debug_headers: bool,
    #[arg(long)]
    pub(crate) panic_file: Option<PathBuf>,
    #[arg(long, help = "disable best-effort latest-version check")]
    pub(crate) no_update_check: bool,
}

#[derive(Debug, Args)]
pub(crate) struct AdminArgs {
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
}

#[derive(Debug, Args)]
pub(crate) struct ExplainArgs {
    pub(crate) route: String,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
}

#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    #[arg(long)]
    pub(crate) config: Option<PathBuf>,
    #[arg(long)]
    pub(crate) to: Option<String>,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
    #[arg(long, help = "disable best-effort latest-version check")]
    pub(crate) no_update_check: bool,
}

#[derive(Debug, Args)]
pub(crate) struct PurgeArgs {
    #[arg(long)]
    pub(crate) all: bool,
    #[arg(long)]
    pub(crate) route: Option<String>,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
    #[arg(long, env = "KUBIO_ADMIN_TOKEN")]
    pub(crate) admin_token: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ConfigSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigSubcommand {
    Check(ConfigCheckArgs),
    Reload(ConfigReloadArgs),
    Diff(ConfigDiffArgs),
    Status(ConfigStatusArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ConfigCheckArgs {
    #[arg(long)]
    pub(crate) config: PathBuf,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigReloadArgs {
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
    #[arg(long)]
    pub(crate) dry_run: bool,
    #[arg(long, env = "KUBIO_ADMIN_TOKEN")]
    pub(crate) admin_token: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigDiffArgs {
    #[arg(long)]
    pub(crate) config: PathBuf,
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
    #[arg(long, env = "KUBIO_ADMIN_TOKEN")]
    pub(crate) admin_token: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ConfigStatusArgs {
    #[arg(long, default_value = "http://127.0.0.1:9900")]
    pub(crate) dashboard: String,
}

#[derive(Debug, Args)]
pub(crate) struct UpdateArgs {
    #[arg(long, help = "check for a newer release without installing it")]
    pub(crate) check: bool,
    #[arg(long, help = "install a specific release tag, such as v0.4.1")]
    pub(crate) version: Option<String>,
    #[arg(long, value_parser = ["standard", "http3-experimental"])]
    pub(crate) flavor: Option<String>,
    #[arg(long)]
    pub(crate) install_dir: Option<PathBuf>,
    #[arg(long, help = "allow updating a development binary under target/")]
    pub(crate) force: bool,
    #[arg(long, hide = true, env = "KUBIO_REPO")]
    pub(crate) repo: Option<String>,
    #[arg(long, hide = true, env = "KUBIO_RELEASE_API_URL")]
    pub(crate) release_api_url: Option<String>,
    #[arg(long, hide = true, env = "KUBIO_DOWNLOAD_BASE_URL")]
    pub(crate) download_base_url: Option<String>,
}
