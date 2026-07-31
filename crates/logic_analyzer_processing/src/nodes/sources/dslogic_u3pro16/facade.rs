use signal_processing::ProcessNode;
use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::platform;

/// Creates the processing source selected for the current platform.
pub fn create_source(
    name: impl Into<String>,
    config: LogicCaptureConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    platform::create_source(name.into(), config)
}
