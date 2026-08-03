use std::path::Path;
use std::sync::Arc;

use signal_processing::Result;

use super::implementation::SigrokFileSource;
use crate::support::capture_archive::FileByteSource;

impl SigrokFileSource {
    /// Temporary path entry point for developer tools and format tests.
    ///
    /// # Parameters
    /// - `path`: Input consumed by this operation.
    pub fn indexed_capture_presentation_from_path(
        path: impl AsRef<Path>,
    ) -> Result<signal_processing::IndexedCapturePresentation> {
        let path = path.as_ref();
        let source = Arc::new(FileByteSource::open(path)?);
        Ok(Self::indexed_capture_presentation(
            source,
            path.display().to_string(),
        ))
    }

    /// Creates a Sigrok source from a host path.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let source = Arc::new(FileByteSource::open(path)?);
        Self::from_prepared_source(source)
    }
}
