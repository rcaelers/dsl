use std::path::Path;
use std::sync::Arc;

use platform_artifacts::PreparedByteSource;
use signal_capture::Result;

use super::archive_work_attribution::DslArchiveWorkAttribution;
use super::source::DslFileSource;
use crate::support::capture_archive::FileByteSource;
use crate::support::capture_index::capture_cache_identity;
use crate::support::dsl_file::DslFileCaptureDataSource;

impl DslFileSource {
    /// Begins opt-in archive-work attribution for a developer-supplied path.
    ///
    /// Keep the returned handle alive while constructing all index, runtime, and presentation
    /// consumers that should contribute to the same source-generation profile.
    pub fn begin_archive_work_attribution_from_path(
        path: impl AsRef<Path>,
    ) -> Result<DslArchiveWorkAttribution> {
        let source = FileByteSource::open(path.as_ref())?;
        Ok(DslArchiveWorkAttribution::begin(source.identity()))
    }

    /// Temporary path entry point for developer tools and format tests.
    ///
    /// # Parameters
    /// - `path`: Input consumed by this operation.
    pub fn indexed_capture_presentation_from_path(
        path: impl AsRef<Path>,
    ) -> Result<signal_capture::IndexedCapturePresentation> {
        let path = path.as_ref();
        let source = Arc::new(FileByteSource::open(path)?);
        Ok(Self::indexed_capture_presentation(
            source,
            path.display().to_string(),
        ))
    }

    /// Temporary path entry point for developer benchmarks.
    pub fn capture_cache_identity(path: impl AsRef<Path>) -> Result<[u8; 32]> {
        let path = path.as_ref();
        let source = Arc::new(FileByteSource::open(path)?);
        let identity = source.identity();
        let source = DslFileCaptureDataSource::open_source(source, path.display().to_string())?;
        Ok(capture_cache_identity(identity, &source))
    }

    /// Creates a DSL source from a host path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let source = Arc::new(FileByteSource::open(path)?);
        Self::from_prepared_source(source, path.display().to_string())
    }
}
