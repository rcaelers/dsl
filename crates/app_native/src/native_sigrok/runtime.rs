use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogScanner, SigrokCatalogSnapshot, SigrokDecoder, SigrokDecoderConfig,
    SigrokDecoderDescriptor, SigrokDecoderRuntime,
};
use platform_runtime::WorkExecutor;
use signal_runtime::ProcessNode;

use super::{PythonSigrokExecutionFactory, discover_sigrok_decoder, scan_catalog};

struct NativeSigrokDecoderRuntime;

impl SigrokDecoderRuntime for NativeSigrokDecoderRuntime {
    fn discover(
        &self,
        decoder_root: &Path,
        decoder_id: &str,
    ) -> Result<SigrokDecoderDescriptor, String> {
        discover_sigrok_decoder(decoder_root.to_owned(), decoder_id)
    }

    fn create(
        &self,
        name: &str,
        config: SigrokDecoderConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<Box<dyn ProcessNode>, String> {
        SigrokDecoder::with_execution_factory(
            config,
            &PythonSigrokExecutionFactory::new(work_executor),
        )
        .map(|decoder| Box::new(decoder.with_name(name)) as Box<dyn ProcessNode>)
    }
}

struct NativeSigrokCatalogScanner;

impl SigrokCatalogScanner for NativeSigrokCatalogScanner {
    fn scan(&self, directories: &[PathBuf]) -> SigrokCatalogSnapshot {
        scan_catalog(directories)
    }
}

pub(crate) fn decoder_runtime() -> Arc<dyn SigrokDecoderRuntime> {
    static RUNTIME: OnceLock<Arc<NativeSigrokDecoderRuntime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| Arc::new(NativeSigrokDecoderRuntime))
        .clone()
}

pub(crate) fn catalog_scanner() -> Arc<dyn SigrokCatalogScanner> {
    static SCANNER: OnceLock<Arc<NativeSigrokCatalogScanner>> = OnceLock::new();
    SCANNER
        .get_or_init(|| Arc::new(NativeSigrokCatalogScanner))
        .clone()
}
