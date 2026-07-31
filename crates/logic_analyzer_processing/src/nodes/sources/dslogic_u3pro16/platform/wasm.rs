use std::sync::Arc;

use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::super::facade::DsLogicU3Pro16SourceFactory;
use crate::ProcessNodeConstruction;
use crate::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;

struct WasmDsLogicU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for WasmDsLogicU3Pro16SourceFactory {
    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        Ok(ProcessNodeConstruction::new(
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.input_mask.count_ones() as usize)
                    .with_name(name),
            ),
            (),
        ))
    }
}

pub(crate) fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    Arc::new(WasmDsLogicU3Pro16SourceFactory)
}
