//! Reproducible end-to-end performance baseline and comparison runner.

mod cli;
mod comparison;
mod model;
mod process;
mod runner;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    runner::run(cli::arguments())
}
