use std::path::Path;
use std::sync::Arc;

use logic_analyzer_graph_api::node_support::CapturePresentation;
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSource, SigrokFileSourceConfig, create_source,
};
use signal_processing::ProcessNode;

use super::builder::SigrokFileArtifacts;

#[derive(Default)]
struct NativeSigrokFileArtifacts {
    identities: super::super::file_identity_cache::FileIdentityCache,
}

impl SigrokFileArtifacts for NativeSigrokFileArtifacts {
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
        path: &Path,
        _channel_names: &[String],
    ) -> Result<Option<CapturePresentation>, String> {
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        let indexed = SigrokFileSource::indexed_capture_presentation(path);
        Ok(Some(CapturePresentation::Indexed {
            identity: indexed.identity,
            factory: indexed.factory,
        }))
    }

    fn cache_identity(&self, path: &Path) -> Result<[u8; 32], String> {
        self.identities.resolve(path, |path| {
            SigrokFileSource::capture_cache_identity(path).map_err(|error| error.to_string())
        })
    }
}

pub(crate) fn artifacts() -> Arc<dyn SigrokFileArtifacts> {
    Arc::new(NativeSigrokFileArtifacts::default())
}

pub(crate) fn channel_names(path: &Path) -> Result<Option<Vec<String>>, String> {
    SigrokFileSource::new(path)
        .map(|source| Some(source.header().probe_names.clone()))
        .map_err(|error| error.to_string())
}
