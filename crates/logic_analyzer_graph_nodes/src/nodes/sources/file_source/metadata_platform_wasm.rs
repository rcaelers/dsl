use std::path::Path;
use std::sync::Arc;

use logic_analyzer_graph_api::node_support::CapturePresentation;
use logic_analyzer_processing::nodes::sources::dsl_file::{DslFileSourceConfig, create_source};
use signal_processing::ProcessNode;

use super::builder::DslFileArtifacts;

#[derive(Default)]
struct WasmDslFileArtifacts;

impl DslFileArtifacts for WasmDslFileArtifacts {
    fn open(
        &self,
        name: &str,
        path: &Path,
        channel_count: usize,
    ) -> Result<Box<dyn ProcessNode>, String> {
        create_source(name, DslFileSourceConfig::new(path, channel_count))
    }

    fn capture_presentation(
        &self,
        _path: &Path,
        channel_names: &[String],
    ) -> Result<Option<CapturePresentation>, String> {
        Ok(Some(
            super::super::synthetic_presentation::capture_presentation(
                channel_names.iter().cloned(),
            ),
        ))
    }

    fn cache_identity(&self, _path: &Path) -> Result<[u8; 32], String> {
        Err("browser synthetic captures have no file identity".to_owned())
    }
}

pub(crate) fn artifacts() -> Arc<dyn DslFileArtifacts> {
    Arc::new(WasmDslFileArtifacts)
}

pub(crate) fn channel_names(_path: &Path) -> Result<Option<Vec<String>>, String> {
    Ok(None)
}
