use signal_processing::ProcessNode;

use super::super::configuration::SigrokFileSourceConfig;
use super::super::implementation::SigrokFileSource;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

pub(crate) fn create_source(
    name: String,
    config: SigrokFileSourceConfig,
) -> Result<Box<dyn ProcessNode>, String> {
    if config.demo_data() {
        return Ok(Box::new(
            SyntheticCaptureSource::new()
                .with_channel_count(config.channel_count())
                .with_name(name),
        ));
    }
    SigrokFileSource::new(config.path())
        .map(|source| Box::new(source.with_name(name)) as Box<dyn ProcessNode>)
        .map_err(|error| error.to_string())
}
