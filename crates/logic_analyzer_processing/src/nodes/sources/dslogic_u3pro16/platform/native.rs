use std::sync::Arc;

use signal_processing::logic_analyzer::LogicCaptureConfig;

use super::super::facade::DsLogicU3Pro16SourceFactory;
use super::super::source::DsLogicU3Pro16Source;
use crate::ProcessNodeConstruction;

struct NativeDsLogicU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for NativeDsLogicU3Pro16SourceFactory {
    fn create(
        &self,
        name: &str,
        config: LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        DsLogicU3Pro16Source::open_first(config)
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), ()))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    Arc::new(NativeDsLogicU3Pro16SourceFactory)
}
