//! Host-injected discovery and execution for Sigrok decoder packages.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use platform_runtime::WorkExecutor;
use signal_runtime::ProcessNode;

use super::contracts::{SigrokCatalogSnapshot, SigrokDecoderDescriptor};
use super::discovery_error::{SigrokCatalogError, SigrokDecoderDiscoveryError};
use super::execution_error::SigrokExecutionStartError;
use super::implementation::SigrokDecoderConfig;

/// Failure reported by a host-provided Sigrok decoder runtime.
#[derive(Clone, Debug, Error)]
pub enum SigrokDecoderRuntimeError {
    /// The saved decoder package could not be discovered or inspected.
    #[error("Sigrok decoder discovery failed: {0}")]
    Discovery(#[source] SigrokDecoderDiscoveryError),
    /// The portable decoder configuration is invalid.
    #[error("Invalid Sigrok decoder configuration: {0}")]
    Configuration(String),
    /// The host execution worker could not be started.
    #[error("Sigrok decoder execution startup failed: {0}")]
    ExecutionStart(#[source] SigrokExecutionStartError),
}

/// Discovers and executes installed Sigrok decoder packages.
pub trait SigrokDecoderRuntime: Send + Sync {
    /// Discovers one saved decoder package and its current contract.
    fn discover(
        &self,
        decoder_root: &Path,
        decoder_id: &str,
    ) -> Result<SigrokDecoderDescriptor, SigrokDecoderRuntimeError>;

    /// Creates one configured decoder processing node.
    fn create(
        &self,
        name: &str,
        config: SigrokDecoderConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<Box<dyn ProcessNode>, SigrokDecoderRuntimeError>;
}

/// Scans host-selected directories for installed Sigrok decoder packages.
pub trait SigrokCatalogScanner: Send + Sync {
    /// Returns the current decoder catalog for the selected directories.
    fn scan(&self, directories: &[PathBuf]) -> Result<SigrokCatalogSnapshot, SigrokCatalogError>;
}
