use std::sync::Arc;

use super::super::configuration::DslFileSourceConfig;
use super::super::facade::DslFileSourceFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

struct WasmDslFileSourceFactory;

impl DslFileSourceFactory for WasmDslFileSourceFactory {
    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
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

pub(crate) fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    Arc::new(WasmDslFileSourceFactory)
}
