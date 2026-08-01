//! Host-facing construction of concrete node-runtime overrides.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use logic_analyzer_graph_api::node::RuntimeBuilderOverride;
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogSnapshot, SigrokDecoderConfig, SigrokDecoderDescriptor,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16SourceFactory;
use signal_processing::{ProcessNode, WorkExecutor};

/// Host-provided discovery and execution for Sigrok decoder packages.
pub trait SigrokDecoderRuntime: Send + Sync {
    /// Discovers the saved decoder package and its current contract.
    fn discover(
        &self,
        decoder_root: &std::path::Path,
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

/// Host-provided scanner for Sigrok decoder-package directories.
pub trait SigrokCatalogScanner: Send + Sync {
    /// Returns the current decoder catalog for the selected directories.
    fn scan(&self, directories: &[PathBuf]) -> SigrokCatalogSnapshot;
}

struct UnavailableSigrokCatalogScanner;

impl SigrokCatalogScanner for UnavailableSigrokCatalogScanner {
    fn scan(&self, _directories: &[PathBuf]) -> SigrokCatalogSnapshot {
        SigrokCatalogSnapshot::default()
    }
}

static SIGROK_CATALOG_SCANNER: OnceLock<Arc<dyn SigrokCatalogScanner>> = OnceLock::new();

/// Installs the host scanner used by existing Sigrok decoder nodes.
pub fn install_sigrok_catalog_scanner(scanner: Arc<dyn SigrokCatalogScanner>) {
    let _ = SIGROK_CATALOG_SCANNER.set(scanner);
}

pub(crate) fn sigrok_catalog_scanner() -> Arc<dyn SigrokCatalogScanner> {
    SIGROK_CATALOG_SCANNER
        .get_or_init(|| Arc::new(UnavailableSigrokCatalogScanner))
        .clone()
}

/// Returns the U3Pro16 builder override for one host-selected source factory.
pub fn u3pro16_runtime_builder_override(
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
) -> RuntimeBuilderOverride {
    crate::nodes::sources::dslogic_u3pro16::runtime_builder_override(source_factory)
}

/// Returns the Sigrok decoder builder override for one host runtime.
pub fn sigrok_decoder_runtime_builder_override(
    runtime: Arc<dyn SigrokDecoderRuntime>,
) -> RuntimeBuilderOverride {
    crate::nodes::decoders::sigrok_decoder::runtime_builder_override(runtime)
}

/// Builds graph-node templates from portable Sigrok discovery metadata.
pub fn sigrok_node_templates(snapshot: &SigrokCatalogSnapshot) -> Vec<node_graph::NodeTemplate> {
    crate::nodes::decoders::sigrok_decoder::node_templates(snapshot)
}
