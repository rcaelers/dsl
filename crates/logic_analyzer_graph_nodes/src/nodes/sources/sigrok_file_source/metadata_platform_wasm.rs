use std::path::Path;
use std::sync::Arc;

use logic_analyzer_graph_api::node_support::CapturePresentation;

use super::builder::SigrokFileArtifacts;

#[derive(Default)]
struct WasmSigrokFileArtifacts;

impl SigrokFileArtifacts for WasmSigrokFileArtifacts {
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
