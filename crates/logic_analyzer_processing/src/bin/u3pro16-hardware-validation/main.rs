//! Explicit U3Pro16 hardware validation command.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        mod native {
            use std::path::PathBuf;

            use clap::{Parser, Subcommand};

            use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::{
                validate_capture_hardware, validate_fpga_hardware,
            };

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

            pub(crate) fn main() -> Result<(), String> {
                match Args::parse().command {
                    Command::Fpga { image } => validate_fpga_hardware(&image),
                    Command::Capture => validate_capture_hardware(),
                }
            }
        }

        fn main() -> Result<(), String> {
            native::main()
        }
    }
}
