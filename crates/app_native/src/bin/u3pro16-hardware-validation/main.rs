//! Explicit U3Pro16 hardware validation command.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use logic_analyzer_device_dslogic::{validate_capture_hardware, validate_fpga_hardware};

#[allow(dead_code)]
#[path = "../../u3pro16_host.rs"]
mod u3pro16_host;

#[derive(Debug, Parser)]
#[command(about = "Run explicit validations against a connected DSLogic U3Pro16")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Load an FPGA image and verify HDL version 0x0e.
    Fpga { image: PathBuf },
    /// Capture 1,024 samples and verify trigger-header ordering.
    Capture,
}

fn main() -> Result<(), String> {
    match Args::parse().command {
        Command::Fpga { image } => {
            validate_fpga_hardware(u3pro16_host::transport_factory().as_ref(), &image)
        }
        Command::Capture => validate_capture_hardware(u3pro16_host::transport_factory().as_ref()),
    }
}
