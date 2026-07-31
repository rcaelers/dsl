use std::path::Path;
use std::sync::Arc;

use logic_analyzer_graph_api::node_support::CapturePresentation;
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSource;

use super::builder::DslFileArtifacts;

#[derive(Default)]
struct NativeDslFileArtifacts {
    identities: super::super::file_identity_cache::FileIdentityCache,
}

impl DslFileArtifacts for NativeDslFileArtifacts {
    fn capture_presentation(
        &self,
        path: &Path,
        _channel_names: &[String],
    ) -> Result<Option<CapturePresentation>, String> {
        if path.as_os_str().is_empty() {
            return Ok(None);
        }
        let indexed = DslFileSource::indexed_capture_presentation(path);
        Ok(Some(CapturePresentation::Indexed {
            identity: indexed.identity,
            factory: indexed.factory,
        }))
    }

    fn cache_identity(&self, path: &Path) -> Result<[u8; 32], String> {
        self.identities.resolve(path, |path| {
            DslFileSource::capture_cache_identity(path).map_err(|error| error.to_string())
        })
    }
}

pub(crate) fn artifacts() -> Arc<dyn DslFileArtifacts> {
    Arc::new(NativeDslFileArtifacts::default())
}

pub(crate) fn channel_names(path: &Path) -> Result<Option<Vec<String>>, String> {
    DslFileSource::new(path)
        .map(|source| Some(source.header().probe_names.clone()))
        .map_err(|error| error.to_string())
}
