use signal_processing::ProcessNode;
use signal_processing::logic_analyzer::LogicCaptureConfig;

use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

pub(crate) fn create_source(
    name: String,
    config: LogicCaptureConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    Ok(Box::new(
        SyntheticCaptureSource::new()
            .with_channel_count(config.input_mask.count_ones() as usize)
            .with_name(name),
    ))
}
