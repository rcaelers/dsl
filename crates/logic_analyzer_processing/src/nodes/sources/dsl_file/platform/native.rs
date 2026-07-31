use std::sync::Arc;

use super::super::configuration::DslFileSourceConfig;
use super::super::facade::DslFileSourceFactory;
use super::super::implementation::DslFileSource;
use crate::ProcessNodeConstruction;

struct NativeDslFileSourceFactory;

impl DslFileSourceFactory for NativeDslFileSourceFactory {
    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
    ) -> Result<ProcessNodeConstruction, String> {
        DslFileSource::new(config.path())
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), ()))
            .map_err(|error| error.to_string())
    }
}

pub(crate) fn source_factory() -> Arc<dyn DslFileSourceFactory> {
    Arc::new(NativeDslFileSourceFactory)
}
