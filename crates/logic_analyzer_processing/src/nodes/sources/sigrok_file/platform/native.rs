use std::sync::Arc;

use super::super::configuration::SigrokFileSourceConfig;
use super::super::facade::SigrokFileSourceFactory;
use super::super::implementation::SigrokFileSource;
use crate::ProcessNodeConstruction;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

struct NativeSigrokFileSourceFactory;

impl SigrokFileSourceFactory for NativeSigrokFileSourceFactory {
    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        if config.demo_data() {
            return Ok(ProcessNodeConstruction::new(
                Box::new(
                    SyntheticCaptureSource::new()
                        .with_channel_count(config.channel_count())
                        .with_name(name),
                ),
                (),
            ));
        }
        SigrokFileSource::new(config.path())
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), ()))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(NativeSigrokFileSourceFactory)
}
