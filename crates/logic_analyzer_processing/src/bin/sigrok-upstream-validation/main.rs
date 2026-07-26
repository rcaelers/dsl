//! Explicit compatibility validation against an upstream Sigrok SPI decoder.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        mod native {
            use std::path::PathBuf;

            use clap::{Parser, Subcommand};

            use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
                validate_spi_chunk_boundaries, validate_spi_oracle,
            };

            #[derive(Debug, Parser)]
            #[command(about = "Validate the hosted decoder against an explicit upstream Sigrok tree")]
            struct Args {
                #[command(subcommand)]
                command: Command,
            }

            #[derive(Debug, Subcommand)]
            enum Command {
                /// Compare output across every raw-input chunk boundary.
                ChunkBoundaries { decoder_root: PathBuf },
                /// Compare hosted output with an installed libsigrokdecode C oracle.
                Oracle {
                    decoder_root: PathBuf,
                    #[arg(long, default_value = "libsigrokdecode")]
                    pkg_config: String,
                    #[arg(long, default_value = "cc")]
                    cc: String,
                },
            }

            pub(crate) fn main() -> Result<(), String> {
                match Args::parse().command {
                    Command::ChunkBoundaries { decoder_root } => {
                        validate_spi_chunk_boundaries(&decoder_root)
                    }
                    Command::Oracle {
                        decoder_root,
                        pkg_config,
                        cc,
                    } => validate_spi_oracle(&decoder_root, &pkg_config, &cc),
                }
            }
        }

        fn main() -> Result<(), String> {
            native::main()
        }
    }
}
