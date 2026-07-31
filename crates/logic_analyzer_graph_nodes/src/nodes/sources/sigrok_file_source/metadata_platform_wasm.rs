use std::path::Path;
use std::sync::Arc;

use logic_analyzer_graph_api::node_support::CapturePresentation;
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSourceConfig, create_source,
};
use signal_processing::ProcessNode;

use super::builder::SigrokFileArtifacts;

#[derive(Default)]
struct WasmSigrokFileArtifacts;

impl SigrokFileArtifacts for WasmSigrokFileArtifacts {
    fn open(
        &self,
        name: &str,
        path: &Path,
        channel_count: usize,
    ) -> Result<Box<dyn ProcessNode>, String> {
        create_source(
            name,
            SigrokFileSourceConfig::new(path, channel_count, false),
        )
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

pub(crate) fn artifacts() -> Arc<dyn SigrokFileArtifacts> {
    Arc::new(WasmSigrokFileArtifacts)
}

pub(crate) fn channel_names(_path: &Path) -> Result<Option<Vec<String>>, String> {
    Ok(None)
}
