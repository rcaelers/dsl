//! Host-injected discovery and execution for Sigrok decoder packages.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use platform_runtime::WorkExecutor;
use signal_runtime::ProcessNode;

use super::contracts::{SigrokCatalogSnapshot, SigrokDecoderDescriptor};
use super::implementation::SigrokDecoderConfig;

/// Discovers and executes installed Sigrok decoder packages.
pub trait SigrokDecoderRuntime: Send + Sync {
    /// Discovers one saved decoder package and its current contract.
    fn discover(
        &self,
        decoder_root: &Path,
        decoder_id: &str,
    ) -> Result<SigrokDecoderDescriptor, String>;

    /// Creates one configured decoder processing node.
    fn create(
        &self,
        name: &str,
        config: SigrokDecoderConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<Box<dyn ProcessNode>, String>;
}

/// Scans host-selected directories for installed Sigrok decoder packages.
pub trait SigrokCatalogScanner: Send + Sync {
    /// Returns the current decoder catalog for the selected directories.
    fn scan(&self, directories: &[PathBuf]) -> SigrokCatalogSnapshot;
}
