use signal_processing::ProcessNode;

use super::super::configuration::SigrokFileSourceConfig;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

pub(crate) fn create_source(
    name: String,
    config: SigrokFileSourceConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    Ok(Box::new(
        SyntheticCaptureSource::new()
            .with_channel_count(config.channel_count())
            .with_name(name),
    ))
}
