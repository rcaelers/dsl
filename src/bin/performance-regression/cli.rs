//! Command-line contract for recording and comparing retained baselines.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(version, about)]
pub(crate) struct Arguments {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Record a retained baseline from one executable.
    Record(RecordArguments),
    /// Compare a candidate with a retained baseline or a freshly measured reference executable.
    Compare(CompareArguments),
}

#[derive(Args)]
pub(crate) struct CommonArguments {
    /// JSON workload specification.
    #[arg(long)]
    pub(crate) workload: PathBuf,
    /// Capture path overriding the workload's environment variable.
    #[arg(long)]
    pub(crate) capture: Option<PathBuf>,
    /// Warmup-run override.
    #[arg(long, value_parser = positive_usize)]
    pub(crate) warmups: Option<usize>,
    /// Measured-run override.
    #[arg(long, value_parser = positive_usize)]
    pub(crate) runs: Option<usize>,
}

#[derive(Args)]
pub(crate) struct RecordArguments {
    #[command(flatten)]
    pub(crate) common: CommonArguments,
    /// Logic Conduit executable to measure.
    #[arg(long)]
    pub(crate) binary: PathBuf,
    /// Baseline JSON path to create.
    #[arg(long)]
    pub(crate) baseline: PathBuf,
    /// Replace an existing retained baseline intentionally.
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Args)]
pub(crate) struct CompareArguments {
    #[command(flatten)]
    pub(crate) common: CommonArguments,
    /// Retained baseline JSON providing expected identities and fallback measurements.
    #[arg(long)]
    pub(crate) baseline: PathBuf,
    /// Candidate Logic Conduit executable.
    #[arg(long)]
    pub(crate) candidate: PathBuf,
    /// Reference executable measured in alternating A/B order with the candidate.
    #[arg(long)]
    pub(crate) reference: Option<PathBuf>,
    /// Optional path for the complete machine-readable comparison report.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
}

pub(crate) fn arguments() -> Arguments {
    Arguments::parse()
}

fn positive_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|error| format!("invalid count: {error}"))
        .and_then(|value| {
            if value == 0 {
                Err("count must be greater than zero".to_owned())
            } else {
                Ok(value)
            }
        })
}

#[cfg(test)]
mod cli_tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn measured_run_counts_must_be_positive() {
        assert!(
            Arguments::try_parse_from([
                "performance-regression",
                "record",
                "--workload",
                "workload.json",
                "--binary",
                "logic-conduit",
                "--baseline",
                "baseline.json",
                "--runs",
                "0",
            ])
            .is_err()
        );
    }
}
