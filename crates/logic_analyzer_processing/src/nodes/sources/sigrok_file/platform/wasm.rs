use std::sync::Arc;

use super::super::configuration::SigrokFileSourceConfig;
use super::super::facade::SigrokFileSourceFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

struct WasmSigrokFileSourceFactory;

impl SigrokFileSourceFactory for WasmSigrokFileSourceFactory {
    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.channel_count())
                    .with_name(name),
            ),
            (),
        ))
    }
}

pub(crate) fn source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(WasmSigrokFileSourceFactory)
}
