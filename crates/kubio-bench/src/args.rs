use clap::{Parser, ValueEnum};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "kubio-bench", about = "Local kubio protocol benchmark runner")]
pub(crate) struct Args {
    #[arg(long, value_enum, default_value_t = Scenario::Smoke)]
    pub(crate) scenario: Scenario,
    #[arg(long, value_enum, default_value_t = BenchProtocol::H1)]
    pub(crate) protocol: BenchProtocol,
    #[arg(long, default_value_t = 20)]
    pub(crate) requests: usize,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub(crate) output: OutputFormat,
    #[arg(long)]
    pub(crate) fail_on_budget: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scenario {
    Smoke,
    FreshHit,
    ExactKeyAdaptive,
    PublicObjectSweep,
    ProtectedUserSweep,
    OriginPublicFastPath,
    QueryNoisyPublicObject,
    SlugPublicObjectSweep,
    SensitiveSlugSweep,
    EvidenceDecay,
    CanaryMismatch,
    DynamicResponseMetadata,
    VendorHeaderCandidate,
    VendorHeaderRouteEnabled,
    ReloadSmoke,
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BenchProtocol {
    H1,
    H2,
    H3,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    Text,
    Json,
}
