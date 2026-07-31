use signal_processing::ProcessNode;

use super::configuration::BinaryFileWriterConfig;
use super::platform;

/// Creates the processing sink selected for the current platform.
pub fn create_writer(
    name: impl Into<String>,
    config: BinaryFileWriterConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    platform::create_writer(name.into(), config)
}
