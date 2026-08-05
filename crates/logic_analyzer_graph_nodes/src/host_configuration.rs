//! Host-facing construction of concrete node-runtime overrides.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use logic_analyzer_graph_capabilities::node::GraphNodeCapabilityOverride;
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogSnapshot, SigrokDecoderConfig, SigrokDecoderDescriptor,
};
use logic_analyzer_processing::nodes::sinks::binary_file_writer::BinaryFileWriterFactory;
use logic_analyzer_processing::nodes::sinks::csv_word_writer::CsvWordWriterFactory;
use logic_analyzer_processing::nodes::sinks::text_file_writer::TextFileWriterFactory;
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSourceFactory, unavailable_source_factory as unavailable_dsl_file_source_factory,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::DsLogicU3Pro16SourceFactory;
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSourceFactory, portable_source_factory as portable_sigrok_file_source_factory,
};
use signal_runtime::{ProcessNode, WorkExecutor};

/// Host-provided discovery and execution for Sigrok decoder packages.
pub trait SigrokDecoderRuntime: Send + Sync {
    /// Discovers the saved decoder package and its current contract.
    ///
    /// # Parameters
    /// - `decoder_root`: Input consumed by this operation.
    /// - `decoder_id`: Input consumed by this operation.
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
static DSL_FILE_SOURCE_FACTORY: OnceLock<RwLock<Arc<dyn DslFileSourceFactory>>> = OnceLock::new();
static SIGROK_FILE_SOURCE_FACTORY: OnceLock<RwLock<Arc<dyn SigrokFileSourceFactory>>> =
    OnceLock::new();

/// Installs the host scanner used by existing Sigrok decoder nodes.
pub fn install_sigrok_catalog_scanner(scanner: Arc<dyn SigrokCatalogScanner>) {
    let _ = SIGROK_CATALOG_SCANNER.set(scanner);
}

pub(crate) fn sigrok_catalog_scanner() -> Arc<dyn SigrokCatalogScanner> {
    SIGROK_CATALOG_SCANNER
        .get_or_init(|| Arc::new(UnavailableSigrokCatalogScanner))
        .clone()
}

/// Installs host acquisition factories used by file-node presentation and runtime overrides.
pub fn install_file_source_factories(
    dsl: Arc<dyn DslFileSourceFactory>,
    sigrok: Arc<dyn SigrokFileSourceFactory>,
) {
    *dsl_file_source_factory_slot().write().unwrap() = dsl;
    *sigrok_file_source_factory_slot().write().unwrap() = sigrok;
}

pub(crate) fn dsl_file_source_factory() -> Arc<dyn DslFileSourceFactory> {
    dsl_file_source_factory_slot().read().unwrap().clone()
}

pub(crate) fn sigrok_file_source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    sigrok_file_source_factory_slot().read().unwrap().clone()
}

fn dsl_file_source_factory_slot() -> &'static RwLock<Arc<dyn DslFileSourceFactory>> {
    DSL_FILE_SOURCE_FACTORY.get_or_init(|| RwLock::new(unavailable_dsl_file_source_factory()))
}

fn sigrok_file_source_factory_slot() -> &'static RwLock<Arc<dyn SigrokFileSourceFactory>> {
    SIGROK_FILE_SOURCE_FACTORY.get_or_init(|| RwLock::new(portable_sigrok_file_source_factory()))
}

/// Returns the U3Pro16 builder override for one host-selected source factory.
pub fn u3pro16_capability_override(
    source_factory: Arc<dyn DsLogicU3Pro16SourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::dslogic_u3pro16::builder::capability_override(source_factory)
}

/// Returns the DSL file-source override for one host acquisition factory.
///
/// # Parameters
/// - `source_factory`: Input consumed by this operation.
pub fn dsl_file_source_capability_override(
    source_factory: Arc<dyn DslFileSourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::file_source::builder::capability_override(source_factory)
}

/// Returns the Sigrok file-source override for one host acquisition factory.
pub fn sigrok_file_source_capability_override(
    source_factory: Arc<dyn SigrokFileSourceFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sources::sigrok_file_source::builder::capability_override(source_factory)
}

/// Returns the binary-file sink override for one host destination factory.
pub fn binary_file_writer_capability_override(
    writer_factory: Arc<dyn BinaryFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::file_writer::builder::capability_override(writer_factory)
}

/// Returns the CSV sink override for one host destination factory.
pub fn csv_word_writer_capability_override(
    writer_factory: Arc<dyn CsvWordWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::csv_writer::builder::capability_override(writer_factory)
}

/// Returns the text-file sink override for one host destination factory.
pub fn text_file_writer_capability_override(
    writer_factory: Arc<dyn TextFileWriterFactory>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::sinks::text_file_writer::builder::capability_override(writer_factory)
}

/// Returns the Sigrok decoder builder override for one host runtime.
pub fn sigrok_decoder_capability_override(
    runtime: Arc<dyn SigrokDecoderRuntime>,
) -> GraphNodeCapabilityOverride {
    crate::nodes::decoders::sigrok_decoder::builder::capability_override(runtime)
}

/// Builds graph-node templates from portable Sigrok discovery metadata.
pub fn sigrok_node_templates(snapshot: &SigrokCatalogSnapshot) -> Vec<node_graph::NodeTemplate> {
    crate::nodes::decoders::sigrok_decoder::definition::node_templates(snapshot)
}
