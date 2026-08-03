use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rusb::{Context, DeviceHandle, UsbContext};

use logic_analyzer_graph_compiler::{
    SourcePreparationExecutor, SourcePreparationResult, SourcePreparationTask,
    SourcePreparationTaskUpdate, SourcePreparationWork,
};
use logic_analyzer_graph_nodes::{
    SigrokCatalogScanner, SigrokDecoderRuntime, install_sigrok_catalog_scanner,
};
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogSnapshot, SigrokDecoder, SigrokDecoderConfig, SigrokDecoderDescriptor,
};
use logic_analyzer_processing::nodes::sinks::{OutputFile, OutputStorage};
use logic_analyzer_processing::nodes::sources::dsl_file::{
    DslFileSource, DslFileSourceConfig, DslFileSourceFactory,
};
use logic_analyzer_processing::nodes::sources::dslogic_u3pro16::{
    DsLogicU3Pro16Capture, DsLogicU3Pro16Source, DsLogicU3Pro16SourceFactory,
    DsLogicU3Pro16TransportFactory, LinkSpeed, UsbError, UsbTransport,
};
use logic_analyzer_processing::nodes::sources::sigrok_file::{
    SigrokFileSource, SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use logic_analyzer_processing::nodes::sources::synthetic_capture_source::SyntheticCaptureSource;
use logic_analyzer_processing::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourcePresentation, CaptureSourceRuntimeCapabilities, ProcessNodeConstruction,
};
use logic_analyzer_ui::{
    APPLICATION_ID, AppServices, ApplicationSettings, DecodedBlockCacheSnapshot, HostCommand,
    HostService, HostUiCapabilities, ModifierKeyLabels, OpenDialog, SaveDialog,
    default_input_bindings,
};
use node_graph::{FileDialogRequest, FileDialogService};
use signal_processing::logic_analyzer::LogicAnalyzerError;
use signal_processing::{
    AppManager, AppManagerBackend, AppManagerFactory, ArtifactRepository, CaptureIndex,
    CaptureIndexBuildProgress, CaptureIndexFactory, IndexedCapturePresentation, PipelineManager,
    PreparedByteSource, ProcessNode, SourceIdentity, WorkExecutor, WorkExecutorTask, WorkTask,
};

use super::native_artifact_repository::NativeArtifactRepository;
use super::native_capture_export::native_capture_export_service;
use super::native_file_identity_cache::NativeFileIdentityCache;
use super::native_file_source::NativeFileByteSource;
use super::native_sigrok;
use super::native_sigrok::{PythonSigrokExecutionFactory, discover_sigrok_decoder, scan_catalog};
use super::native_worker::NativeWorkerOperationExecutor;
use crate::services::PlatformServices;

#[cfg(target_os = "macos")]
type RecentFilesListener = Box<dyn Fn(&[PathBuf]) + Send + Sync>;

#[cfg(target_os = "macos")]
static RECENT_FILES_LISTENER: std::sync::OnceLock<RecentFilesListener> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
/// Sets recent files listener.
///
/// # Parameters
/// - `listener`: Input consumed by this operation.
pub fn set_recent_files_listener(listener: impl Fn(&[PathBuf]) + Send + Sync + 'static) {
    let _ = RECENT_FILES_LISTENER.set(Box::new(listener));
}

struct HostCommandBridge {
    #[cfg(any(target_os = "macos", test))]
    sender: crossbeam_channel::Sender<HostCommand>,
    receiver: crossbeam_channel::Receiver<HostCommand>,
    repaint: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

static HOST_COMMAND_BRIDGE: std::sync::OnceLock<HostCommandBridge> = std::sync::OnceLock::new();

fn host_command_bridge() -> &'static HostCommandBridge {
    HOST_COMMAND_BRIDGE.get_or_init(|| {
        let (_sender, receiver) = crossbeam_channel::unbounded();
        HostCommandBridge {
            #[cfg(any(target_os = "macos", test))]
            sender: _sender,
            receiver,
            repaint: std::sync::Mutex::new(None),
        }
    })
}

#[cfg(target_os = "macos")]
/// Dispatches one host-shell command into the portable application command queue.
pub fn dispatch_host_command(command: HostCommand) {
    queue_host_command(command);
}

#[cfg(any(target_os = "macos", test))]
fn queue_host_command(command: HostCommand) {
    let bridge = host_command_bridge();
    let _ = bridge.sender.send(command);
    if let Some(repaint) = bridge.repaint.lock().unwrap().as_ref() {
        repaint();
    }
}

pub(crate) fn standard_services() -> PlatformServices {
    let cache_directory = derived_cache_directory();
    let artifact_repository: Arc<dyn signal_processing::ArtifactRepository> = Arc::new(
        NativeArtifactRepository::new(cache_directory.join("artifacts")),
    );
    let input_bindings = load_input_bindings();
    let application_settings = load_application_settings();
    let capture_export_service = native_capture_export_service(Arc::clone(&artifact_repository));
    let work_executor: Arc<dyn WorkExecutor> = Arc::new(NativeWorkExecutor::new());
    let settings_path = dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("logic-conduit")
        .join("sigrok_decoders.json");
    let sigrok_catalog_scanner = native_sigrok_catalog_scanner();
    install_sigrok_catalog_scanner(Arc::clone(&sigrok_catalog_scanner));
    let dsl_file_source_factory = native_dsl_file_source_factory();
    let sigrok_file_source_factory = native_sigrok_file_source_factory();
    logic_analyzer_graph_nodes::install_file_source_factories(
        Arc::clone(&dsl_file_source_factory),
        Arc::clone(&sigrok_file_source_factory),
    );
    let node_catalogs = vec![Box::new(native_sigrok::directory_catalog(
        settings_path,
        native_sigrok_decoder_directories(),
        Arc::clone(&work_executor),
    ))
        as Box<dyn logic_analyzer_graph_api::node::DirectoryNodeCatalog>];
    let ui_services = AppServices::with_host_configuration(
        Box::new(NativeHostService::new()),
        input_bindings,
        application_settings,
        system_symbol_fonts(),
    )
    .with_capture_export_service(capture_export_service)
    .with_node_file_dialog(Box::new(NativeNodeFileDialogService))
    .with_graph_execution_and_builder_overrides(
        Box::new(NativeSourcePreparationExecutor::new()),
        Arc::new(NativeAppManagerFactory {
            work_executor: Arc::new(NativeRuntimeExecutor),
        }),
        Arc::clone(&work_executor),
        vec![
            logic_analyzer_graph_nodes::binary_file_writer_runtime_builder_override(
                logic_analyzer_processing::nodes::sinks::binary_file_writer::writer_factory(
                    native_output_storage(),
                ),
            ),
            logic_analyzer_graph_nodes::csv_word_writer_runtime_builder_override(
                logic_analyzer_processing::nodes::sinks::csv_word_writer::writer_factory(
                    native_output_storage(),
                ),
            ),
            logic_analyzer_graph_nodes::text_file_writer_runtime_builder_override(
                logic_analyzer_processing::nodes::sinks::text_file_writer::writer_factory(
                    native_output_storage(),
                ),
            ),
            logic_analyzer_graph_nodes::dsl_file_source_runtime_builder_override(
                dsl_file_source_factory,
            ),
            logic_analyzer_graph_nodes::sigrok_file_source_runtime_builder_override(
                sigrok_file_source_factory,
            ),
            logic_analyzer_graph_nodes::sigrok_decoder_runtime_builder_override(
                native_sigrok_decoder_runtime(),
            ),
            logic_analyzer_graph_nodes::u3pro16_runtime_builder_override(
                native_u3pro16_source_factory(),
            ),
        ],
    );
    PlatformServices::with_ui_services(
        ui_services,
        node_catalogs,
        artifact_repository,
        work_executor,
        Rc::new(NativeWorkerOperationExecutor::new()),
    )
}

struct NativeOutputStorage;

impl OutputStorage for NativeOutputStorage {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        File::create(path).map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

fn native_output_storage() -> Arc<dyn OutputStorage> {
    Arc::new(NativeOutputStorage)
}

const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

struct NativeDslFileSourceMetadata {
    config: DslFileSourceConfig,
    identities: Arc<NativeFileIdentityCache>,
}

fn acquire_native_file(
    path: &Path,
    identities: &NativeFileIdentityCache,
) -> Result<Arc<dyn PreparedByteSource>, String> {
    let identity = identities.resolve(path, |path| {
        NativeFileByteSource::acquire(path)
            .map(|source| *source.identity().as_bytes())
            .map_err(|error| error.to_string())
    })?;
    NativeFileByteSource::open(path, SourceIdentity::from_bytes(identity))
        .map(|source| Arc::new(source) as Arc<dyn PreparedByteSource>)
        .map_err(|error| error.to_string())
}

struct NativeDslCaptureIndexFactory {
    path: PathBuf,
    identities: Arc<NativeFileIdentityCache>,
}

impl CaptureIndexFactory for NativeDslCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn metadata(&self) -> signal_processing::Result<signal_processing::CaptureMetadata> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_processing::Error::ParseError)?;
        DslFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .metadata()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_processing::Error::ParseError)?;
        DslFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

struct NativeSigrokCaptureIndexFactory {
    path: PathBuf,
    identities: Arc<NativeFileIdentityCache>,
}

impl CaptureIndexFactory for NativeSigrokCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn metadata(&self) -> signal_processing::Result<signal_processing::CaptureMetadata> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_processing::Error::ParseError)?;
        SigrokFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .metadata()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_processing::Result<Box<dyn CaptureIndex + Send>> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_processing::Error::ParseError)?;
        SigrokFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .open(artifact_repository, work_executor, progress)
    }
}

impl CaptureSourceMetadata for NativeDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let source = acquire_native_file(self.config.path(), &self.identities)?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: source.identity(),
                factory: Box::new(NativeDslCaptureIndexFactory {
                    path: self.config.path().to_owned(),
                    identities: Arc::clone(&self.identities),
                }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.path().as_os_str().is_empty() {
            return CaptureSourceCacheIdentity::Dynamic;
        }
        acquire_native_file(self.config.path(), &self.identities)
            .map(|source| CaptureSourceCacheIdentity::Stable(*source.identity().as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        acquire_native_file(self.config.path(), &self.identities)
            .and_then(|source| {
                DslFileSource::from_prepared_source(
                    source,
                    self.config.path().display().to_string(),
                )
                .map_err(|error| error.to_string())
            })
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct NativeDslFileSourceFactory {
    identities: Arc<NativeFileIdentityCache>,
}

impl DslFileSourceFactory for NativeDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeDslFileSourceMetadata {
            config,
            identities: Arc::clone(&self.identities),
        })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        artifact_repository: Arc<dyn signal_processing::ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        acquire_native_file(config.path(), &self.identities)
            .and_then(|source| {
                DslFileSource::from_prepared_source(source, config.path().display().to_string())
                    .map_err(|error| error.to_string())
            })
            .map(|source| {
                ProcessNodeConstruction::new(
                    Box::new(
                        source
                            .with_name(name)
                            .with_artifact_repository(artifact_repository)
                            .with_work_executor(work_executor),
                    ),
                    metadata,
                )
            })
    }
}

fn native_dsl_file_source_factory() -> Arc<dyn DslFileSourceFactory> {
    Arc::new(NativeDslFileSourceFactory {
        identities: Arc::new(NativeFileIdentityCache::default()),
    })
}

struct NativeSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
    identities: Arc<NativeFileIdentityCache>,
}

impl CaptureSourceMetadata for NativeSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        if self.config.demo_data() {
            return portable_source_factory()
                .metadata(self.config.clone())
                .presentation();
        }
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let source = acquire_native_file(self.config.path(), &self.identities)?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            IndexedCapturePresentation {
                identity: source.identity(),
                factory: Box::new(NativeSigrokCaptureIndexFactory {
                    path: self.config.path().to_owned(),
                    identities: Arc::clone(&self.identities),
                }),
            },
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            return CaptureSourceCacheIdentity::NotCapture;
        }
        acquire_native_file(self.config.path(), &self.identities)
            .map(|source| CaptureSourceCacheIdentity::Stable(*source.identity().as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        acquire_native_file(self.config.path(), &self.identities)
            .and_then(|source| {
                SigrokFileSource::from_prepared_source(source).map_err(|error| error.to_string())
            })
            .map(|source| Some(source.header().probe_names.clone()))
    }
}

struct NativeSigrokFileSourceFactory {
    identities: Arc<NativeFileIdentityCache>,
}

impl SigrokFileSourceFactory for NativeSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeSigrokFileSourceMetadata {
            config,
            identities: Arc::clone(&self.identities),
        })
    }

    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        let process = if config.demo_data() {
            Box::new(
                SyntheticCaptureSource::new()
                    .with_channel_count(config.channel_count())
                    .with_name(name),
            ) as Box<dyn ProcessNode>
        } else {
            Box::new(
                SigrokFileSource::from_prepared_source(acquire_native_file(
                    config.path(),
                    &self.identities,
                )?)
                .map_err(|error| error.to_string())?
                .with_name(name)
                .with_work_executor(work_executor),
            )
        };
        Ok(ProcessNodeConstruction::new(process, metadata))
    }
}

fn native_sigrok_file_source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(NativeSigrokFileSourceFactory {
        identities: Arc::new(NativeFileIdentityCache::default()),
    })
}

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

fn native_sigrok_decoder_runtime() -> Arc<dyn SigrokDecoderRuntime> {
    static RUNTIME: OnceLock<Arc<NativeSigrokDecoderRuntime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| Arc::new(NativeSigrokDecoderRuntime))
        .clone()
}

struct NativeSigrokCatalogScanner;

impl SigrokCatalogScanner for NativeSigrokCatalogScanner {
    fn scan(&self, directories: &[PathBuf]) -> SigrokCatalogSnapshot {
        scan_catalog(directories)
    }
}

fn native_sigrok_catalog_scanner() -> Arc<dyn SigrokCatalogScanner> {
    static SCANNER: OnceLock<Arc<NativeSigrokCatalogScanner>> = OnceLock::new();
    SCANNER
        .get_or_init(|| Arc::new(NativeSigrokCatalogScanner))
        .clone()
}

fn native_sigrok_decoder_directories() -> Vec<PathBuf> {
    let mut paths = std::env::var_os("SIGROK_DECODERS_DIR")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    for path in [
        PathBuf::from("/opt/homebrew/share/libsigrokdecode/decoders"),
        PathBuf::from("/usr/local/share/libsigrokdecode/decoders"),
        PathBuf::from("/usr/share/libsigrokdecode/decoders"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dslogic/libsigrokdecode/decoders"),
    ] {
        if path.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

const U3PRO16_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true);

struct NativeU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for NativeU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        U3PRO16_LIFECYCLE
    }

    fn metadata(
        &self,
        config: signal_processing::logic_analyzer::LogicCaptureConfig,
    ) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeU3Pro16Metadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: signal_processing::logic_analyzer::LogicCaptureConfig,
    ) -> Result<ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>, String> {
        let metadata = self.metadata(config.clone());
        native_u3pro16_transport_factory()
            .open()
            .and_then(|transport| DsLogicU3Pro16Source::from_transport(config, transport))
            .map(|source| ProcessNodeConstruction::new(Box::new(source.with_name(name)), metadata))
            .map_err(|error| error.to_string())
    }
}

struct NativeU3Pro16Metadata {
    config: signal_processing::logic_analyzer::LogicCaptureConfig,
}

impl NativeU3Pro16Metadata {
    fn enabled_channels(&self) -> impl Iterator<Item = usize> + '_ {
        (0..u64::BITS as usize).filter(|channel| self.config.input_mask & (1_u64 << channel) != 0)
    }
}

impl CaptureSourceMetadata for NativeU3Pro16Metadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        U3PRO16_LIFECYCLE
    }

    fn presentation(&self) -> Result<Option<CaptureSourcePresentation>, String> {
        Ok(Some(CaptureSourcePresentation::Channels(
            self.enabled_channels()
                .enumerate()
                .map(|(viewer_channel, physical_channel)| {
                    (viewer_channel, format!("Ch {physical_channel}"))
                })
                .collect(),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        CaptureSourceCacheIdentity::NotCapture
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, String> {
        Ok(Some(
            self.enabled_channels()
                .map(|channel| format!("Ch {channel}"))
                .collect(),
        ))
    }

    fn runtime_capabilities(&self) -> CaptureSourceRuntimeCapabilities {
        CaptureSourceRuntimeCapabilities::new(true)
    }

    fn configured_acquisition(
        &self,
    ) -> Result<Option<Box<dyn signal_processing::ConfiguredAcquisition>>, String> {
        let channels = self
            .enabled_channels()
            .map(|channel| {
                signal_processing::CaptureChannelId::new(format!("u3pro16:input:{channel}"))
            })
            .collect::<Vec<_>>();
        DsLogicU3Pro16Capture::new(
            self.config.clone(),
            channels,
            native_u3pro16_transport_factory(),
        )
        .map(|capture| Some(Box::new(capture) as Box<dyn signal_processing::ConfiguredAcquisition>))
        .map_err(|error| error.to_string())
    }
}

fn native_u3pro16_source_factory() -> Arc<dyn DsLogicU3Pro16SourceFactory> {
    static FACTORY: OnceLock<Arc<NativeU3Pro16SourceFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| Arc::new(NativeU3Pro16SourceFactory))
        .clone()
}

struct NativeU3Pro16TransportFactory;

impl DsLogicU3Pro16TransportFactory for NativeU3Pro16TransportFactory {
    fn open(
        &self,
    ) -> signal_processing::logic_analyzer::LogicAnalyzerResult<Box<dyn UsbTransport>> {
        NativeU3Pro16Transport::open_first().map(|transport| Box::new(transport) as Box<_>)
    }
}

pub(crate) fn native_u3pro16_transport_factory() -> Arc<dyn DsLogicU3Pro16TransportFactory> {
    static FACTORY: OnceLock<Arc<NativeU3Pro16TransportFactory>> = OnceLock::new();
    FACTORY
        .get_or_init(|| Arc::new(NativeU3Pro16TransportFactory))
        .clone()
}

const U3PRO16_VENDOR_ID: u16 = 0x2a0e;
const U3PRO16_PRODUCT_ID: u16 = 0x002a;
const U3PRO16_RUNTIME_MANUFACTURER: &str = "DreamSourceLab";
const U3PRO16_RUNTIME_PRODUCT: &str = "USB-based DSL Instrument v2";
const U3PRO16_CANCELLATION_TIMEOUT: Duration = Duration::from_millis(1_000);

/// Native `rusb` adapter for the U3Pro16 transport capability contract.
struct NativeU3Pro16Transport {
    context: Context,
    handle: DeviceHandle<Context>,
    speed: LinkSpeed,
    claimed: bool,
    queued_bulk_reads: VecDeque<QueuedBulkRead>,
}

struct QueuedBulkRead {
    transfer: *mut rusb::ffi::libusb_transfer,
    buffer: Box<[u8]>,
    complete: Box<AtomicBool>,
}

// A single capture worker owns each native transport and its queued requests.
unsafe impl Send for QueuedBulkRead {}

extern "system" fn mark_bulk_read_complete(transfer: *mut rusb::ffi::libusb_transfer) {
    // SAFETY: `user_data` points to the completion flag owned by the queued
    // request until that completed request is freed.
    unsafe {
        let complete = (*transfer).user_data.cast::<AtomicBool>();
        (*complete).store(true, Ordering::Release);
    }
}

impl NativeU3Pro16Transport {
    fn open_first() -> signal_processing::logic_analyzer::LogicAnalyzerResult<Self> {
        let context = Context::new().map_err(native_rusb_error)?;
        let devices = context.devices().map_err(native_rusb_error)?;
        for device in devices.iter() {
            let descriptor = device.device_descriptor().map_err(native_rusb_error)?;
            if descriptor.vendor_id() != U3PRO16_VENDOR_ID
                || descriptor.product_id() != U3PRO16_PRODUCT_ID
            {
                continue;
            }
            let speed = match device.speed() {
                rusb::Speed::High => LinkSpeed::High,
                rusb::Speed::Super => LinkSpeed::Super,
                _ => continue,
            };
            let handle = device.open().map_err(native_rusb_error)?;
            let manufacturer = handle
                .read_manufacturer_string_ascii(&descriptor)
                .map_err(native_rusb_error)?;
            let product = handle
                .read_product_string_ascii(&descriptor)
                .map_err(native_rusb_error)?;
            if !manufacturer.starts_with(U3PRO16_RUNTIME_MANUFACTURER)
                || !product.starts_with(U3PRO16_RUNTIME_PRODUCT)
            {
                continue;
            }
            if handle.active_configuration().map_err(native_rusb_error)? != 1 {
                handle
                    .set_active_configuration(1)
                    .map_err(native_rusb_error)?;
            }
            if handle.kernel_driver_active(0).unwrap_or(false) {
                let _ = handle.detach_kernel_driver(0);
            }
            handle.claim_interface(0).map_err(native_rusb_error)?;
            return Ok(Self {
                context,
                handle,
                speed,
                claimed: true,
                queued_bulk_reads: VecDeque::new(),
            });
        }
        Err(LogicAnalyzerError::Transport(
            "no accessible DSLogic U3Pro16 runtime device found".into(),
        ))
    }
}

impl UsbTransport for NativeU3Pro16Transport {
    fn link_speed(&self) -> LinkSpeed {
        self.speed
    }

    fn fpga_image(
        &self,
    ) -> signal_processing::logic_analyzer::LogicAnalyzerResult<Option<Vec<u8>>> {
        let mut candidates = vec![
            PathBuf::from("DSLogicU3Pro16.bin"),
            PathBuf::from("firmware/DSLogicU3Pro16.bin"),
            PathBuf::from("/Applications/DSView.app/Contents/MacOS/res/DSLogicU3Pro16.bin"),
            PathBuf::from("/Applications/DSView.app/Contents/Resources/driver/DSLogicU3Pro16.bin"),
            PathBuf::from("/usr/share/DSView/driver/DSLogicU3Pro16.bin"),
            PathBuf::from("/usr/local/share/DSView/driver/DSLogicU3Pro16.bin"),
        ];
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            candidates.push(home.join(".local/share/DSView/driver/DSLogicU3Pro16.bin"));
            candidates
                .push(home.join("Library/Application Support/DSView/driver/DSLogicU3Pro16.bin"));
        }
        if let Some(path) = std::env::var_os("DSLOGIC_U3PRO16_FPGA_IMAGE") {
            candidates.push(PathBuf::from(path));
        }

        let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
            return Ok(None);
        };
        let image = std::fs::read(&path).map_err(|error| {
            LogicAnalyzerError::Transport(format!(
                "cannot read U3Pro16 FPGA image '{}': {error}",
                path.display()
            ))
        })?;
        tracing::info!(path = %path.display(), "loaded DSLogic U3Pro16 FPGA image");
        Ok(Some(image))
    }

    fn control_write(
        &mut self,
        ty: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.handle
            .write_control(ty, request, value, index, data, timeout)
            .map_err(native_usb_error)
    }

    fn control_read(
        &mut self,
        ty: u8,
        request: u8,
        value: u16,
        index: u16,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.handle
            .read_control(ty, request, value, index, data, timeout)
            .map_err(native_usb_error)
    }

    fn bulk_write(
        &mut self,
        endpoint: u8,
        data: &[u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.handle
            .write_bulk(endpoint, data, timeout)
            .map_err(native_usb_error)
    }

    fn bulk_read(
        &mut self,
        endpoint: u8,
        data: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, UsbError> {
        self.handle
            .read_bulk(endpoint, data, timeout)
            .map_err(native_usb_error)
    }

    fn queue_bulk_read(
        &mut self,
        endpoint: u8,
        byte_len: usize,
        _timeout: Duration,
    ) -> Result<bool, UsbError> {
        if self.queued_bulk_reads.len() == 8 {
            return Err(UsbError::Other);
        }
        let mut buffer = vec![0; byte_len].into_boxed_slice();
        let complete = Box::new(AtomicBool::new(false));
        // SAFETY: the request and all referenced allocations stay owned by
        // `QueuedBulkRead` until the request completes or is cancelled.
        let transfer = unsafe { rusb::ffi::libusb_alloc_transfer(0) };
        if transfer.is_null() {
            return Err(UsbError::Other);
        }
        unsafe {
            rusb::ffi::libusb_fill_bulk_transfer(
                transfer,
                self.handle.as_raw(),
                endpoint,
                buffer.as_mut_ptr(),
                i32::try_from(byte_len).map_err(|_| UsbError::Other)?,
                mark_bulk_read_complete,
                (&raw const *complete).cast_mut().cast(),
                // A trigger header can arrive later, so completion is polled
                // by `take_queued_bulk_read` rather than timing out here.
                0,
            );
            if rusb::ffi::libusb_submit_transfer(transfer) != 0 {
                rusb::ffi::libusb_free_transfer(transfer);
                return Err(UsbError::Other);
            }
        }
        self.queued_bulk_reads.push_back(QueuedBulkRead {
            transfer,
            buffer,
            complete,
        });
        tracing::debug!(endpoint, byte_len, "queued U3Pro16 bulk receive");
        Ok(true)
    }

    fn take_queued_bulk_read(
        &mut self,
        byte_len: usize,
        timeout: Duration,
    ) -> Result<Option<Vec<u8>>, UsbError> {
        if !self
            .queued_bulk_reads
            .iter()
            .any(|queued| queued.buffer.len() == byte_len)
        {
            tracing::debug!("no queued U3Pro16 bulk receive was available");
            return Ok(None);
        }
        let deadline = Instant::now() + timeout;
        let queued_index = loop {
            if let Some(index) = self.queued_bulk_reads.iter().position(|queued| {
                queued.buffer.len() == byte_len && queued.complete.load(Ordering::Acquire)
            }) {
                break index;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(UsbError::Timeout);
            }
            self.context
                .handle_events(Some(remaining))
                .map_err(native_usb_error)?;
        };
        let queued = self
            .queued_bulk_reads
            .remove(queued_index)
            .expect("queued U3Pro16 receive exists");
        // SAFETY: completion was observed, so libusb no longer accesses this
        // request or its buffer.
        let (status, actual_length) =
            unsafe { ((*queued.transfer).status, (*queued.transfer).actual_length) };
        unsafe { rusb::ffi::libusb_free_transfer(queued.transfer) };
        if status != rusb::constants::LIBUSB_TRANSFER_COMPLETED || actual_length < 0 {
            return Err(if status == rusb::constants::LIBUSB_TRANSFER_TIMED_OUT {
                UsbError::Timeout
            } else {
                UsbError::Other
            });
        }
        let actual_length = usize::try_from(actual_length).map_err(|_| UsbError::Other)?;
        if actual_length > queued.buffer.len() {
            return Err(UsbError::Other);
        }
        let mut buffer = queued.buffer.into_vec();
        buffer.truncate(actual_length);
        Ok(Some(buffer))
    }

    fn cancel_queued_bulk_read(&mut self) -> Result<(), UsbError> {
        while let Some(queued) = self.queued_bulk_reads.pop_front() {
            if !queued.complete.load(Ordering::Acquire) {
                // SAFETY: this transport is the sole owner of the request.
                if unsafe { rusb::ffi::libusb_cancel_transfer(queued.transfer) } != 0 {
                    // libusb can still access the request after a failed cancel.
                    std::mem::forget(queued);
                    return Err(UsbError::Other);
                }
                let deadline = Instant::now() + U3PRO16_CANCELLATION_TIMEOUT;
                while !queued.complete.load(Ordering::Acquire) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        // Do not free memory that libusb may still access.
                        std::mem::forget(queued);
                        return Err(UsbError::Timeout);
                    }
                    if self.context.handle_events(Some(remaining)).is_err() {
                        std::mem::forget(queued);
                        return Err(UsbError::Other);
                    }
                }
            }
            unsafe { rusb::ffi::libusb_free_transfer(queued.transfer) };
        }
        Ok(())
    }

    fn close(&mut self) -> Result<(), UsbError> {
        self.cancel_queued_bulk_read()?;
        if self.claimed {
            self.handle.release_interface(0).map_err(native_usb_error)?;
            self.claimed = false;
        }
        Ok(())
    }
}

impl Drop for NativeU3Pro16Transport {
    fn drop(&mut self) {
        let _ = self.cancel_queued_bulk_read();
    }
}

fn native_usb_error(error: rusb::Error) -> UsbError {
    if error == rusb::Error::Timeout {
        UsbError::Timeout
    } else {
        UsbError::Other
    }
}

fn native_rusb_error(error: rusb::Error) -> LogicAnalyzerError {
    LogicAnalyzerError::Transport(error.to_string())
}

struct NativeWorkExecutor {
    sender: crossbeam_channel::Sender<WorkExecutorTask>,
    workers: usize,
}

impl NativeWorkExecutor {
    fn new() -> Self {
        let workers = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
            // An index preparation task can submit bounded block work to the
            // same host executor. Keep one worker available for that nested
            // work even on single-core hosts.
            .clamp(2, 32);
        let (sender, receiver) = crossbeam_channel::bounded(workers * 4);
        for index in 0..workers {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("processing-work-{index}"))
                .spawn(move || run_work_executor_worker(receiver))
                .expect("failed to start processing work executor");
        }
        Self { sender, workers }
    }
}

impl WorkExecutor for NativeWorkExecutor {
    fn available_parallelism(&self) -> usize {
        self.workers
    }

    fn supports_long_running_tasks(&self) -> bool {
        true
    }

    fn idle(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_completed = Arc::clone(&completed);
        let (completion_sender, completion_receiver) = crossbeam_channel::bounded(1);
        self.sender
            .try_send(Box::new(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
                task_completed.store(true, Ordering::Release);
                let _ = completion_sender.send(());
            }))
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => {
                    String::from("processing work executor queue is full")
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    String::from("processing work executor stopped")
                }
            })?;
        Ok(Box::new(NativeWorkTask {
            completed,
            completion_receiver,
        }))
    }

    fn submit_long_running(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        spawn_runtime_task(task)
    }
}

struct NativeWorkTask {
    completed: Arc<std::sync::atomic::AtomicBool>,
    completion_receiver: crossbeam_channel::Receiver<()>,
}

/// Host runtime executor for long-lived node and watchdog supervision.
///
/// Runtime tasks may block on stream endpoints, so they deliberately do not
/// share the bounded worker queue used for finite decoding and indexing work.
struct NativeRuntimeExecutor;

impl WorkExecutor for NativeRuntimeExecutor {
    fn available_parallelism(&self) -> usize {
        1
    }

    fn supports_long_running_tasks(&self) -> bool {
        true
    }

    fn idle(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
        spawn_runtime_task(task)
    }
}

fn spawn_runtime_task(task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
    let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task_completed = Arc::clone(&completed);
    let (completion_sender, completion_receiver) = crossbeam_channel::bounded(1);
    std::thread::Builder::new()
        .name("processing-runtime".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
            task_completed.store(true, Ordering::Release);
            let _ = completion_sender.send(());
        })
        .map_err(|error| error.to_string())?;
    Ok(Box::new(NativeWorkTask {
        completed,
        completion_receiver,
    }))
}

impl WorkTask for NativeWorkTask {
    fn is_finished(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn wait(self: Box<Self>) {
        let _ = self.completion_receiver.recv();
    }
}

fn run_work_executor_worker(receiver: crossbeam_channel::Receiver<WorkExecutorTask>) {
    while let Ok(task) = receiver.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task));
    }
}

struct NativeAppManagerFactory {
    work_executor: Arc<dyn WorkExecutor>,
}

impl AppManagerFactory for NativeAppManagerFactory {
    fn create(&self) -> AppManager {
        AppManager::with_backend(Box::new(NativeAppManagerBackend {
            manager: PipelineManager::new(Arc::clone(&self.work_executor)),
        }))
    }
}

struct NativeAppManagerBackend {
    manager: PipelineManager,
}

impl AppManagerBackend for NativeAppManagerBackend {
    fn is_finished(&self) -> bool {
        self.manager.is_finished()
    }

    fn add_node(&mut self, spec: signal_processing::NodeSpec) -> Result<(), String> {
        self.manager.add_node(spec)
    }

    fn add_node_deferred(&mut self, spec: signal_processing::NodeSpec) -> Result<(), String> {
        self.manager.add_node_deferred(spec)
    }

    fn start_all_deferred(&mut self) -> Result<(), String> {
        self.manager.start_all_deferred()
    }

    fn remove_node(&mut self, name: &str) -> Result<(), String> {
        self.manager.remove_node(name)
    }

    fn reconfigure(
        &mut self,
        name: &str,
        config: signal_processing::NodeConfig,
    ) -> Result<(), String> {
        self.manager.reconfigure(name, config)
    }

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: signal_processing::NodeConfig,
        boundary: signal_processing::ConfigurationBoundary,
    ) -> Result<(), String> {
        self.manager.reconfigure_at(name, config, boundary)
    }

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn signal_processing::ProcessNode>,
        inputs: Vec<Option<signal_processing::InputSub>>,
    ) -> Result<(), String> {
        self.manager.restart_node(name, node, inputs)
    }

    fn progress(&self) -> Vec<(String, u64)> {
        self.manager.progress()
    }

    fn take_disconnected(&self) -> Vec<signal_processing::DisconnectEvent> {
        self.manager.take_disconnected()
    }

    fn take_failures(&mut self) -> Vec<signal_processing::NodeFailure> {
        self.manager.take_failures()
    }

    fn request_stop(&mut self) {
        self.manager.request_stop();
    }

    fn wait(&mut self) {
        self.manager.wait();
    }

    fn pump(&mut self, budget: usize) {
        self.manager.pump(budget);
    }
}

struct NativeSourcePreparationExecutor {
    sender: crossbeam_channel::Sender<QueuedSourcePreparation>,
}

impl NativeSourcePreparationExecutor {
    fn new() -> Self {
        const WORKERS: usize = 1;
        let (sender, receiver) = crossbeam_channel::bounded(WORKERS * 2);
        for index in 0..WORKERS {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("source-preparation-{index}"))
                .spawn(move || run_source_preparation_worker(receiver))
                .expect("failed to start source preparation worker");
        }
        Self { sender }
    }
}

impl SourcePreparationExecutor for NativeSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
        control: logic_analyzer_graph_compiler::SourcePreparationControl,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        self.sender
            .try_send(QueuedSourcePreparation {
                work,
                control,
                result_sender: sender,
            })
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => {
                    String::from("source-preparation worker queue is full")
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    String::from("source-preparation worker stopped")
                }
            })?;
        Ok(Box::new(NativeSourcePreparationTask { receiver }))
    }
}

struct QueuedSourcePreparation {
    work: SourcePreparationWork,
    control: logic_analyzer_graph_compiler::SourcePreparationControl,
    result_sender: crossbeam_channel::Sender<SourcePreparationResult>,
}

fn run_source_preparation_worker(receiver: crossbeam_channel::Receiver<QueuedSourcePreparation>) {
    while let Ok(QueuedSourcePreparation {
        work,
        control,
        result_sender,
    }) = receiver.recv()
    {
        let result = if control.is_cancelled() {
            Err("source preparation cancelled".into())
        } else {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(control)))
                .unwrap_or_else(|_| Err("source-preparation worker panicked".into()))
        };
        let _ = result_sender.send(result);
    }
}

struct NativeSourcePreparationTask {
    receiver: crossbeam_channel::Receiver<SourcePreparationResult>,
}

impl SourcePreparationTask for NativeSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        match self.receiver.try_recv() {
            Ok(result) => SourcePreparationTaskUpdate::Complete(result),
            Err(crossbeam_channel::TryRecvError::Empty) => SourcePreparationTaskUpdate::Pending,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                SourcePreparationTaskUpdate::Disconnected
            }
        }
    }
}

fn system_symbol_fonts() -> Vec<egui::FontData> {
    symbol_font_paths()
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .map(egui::FontData::from_owned)
        .collect()
}

#[cfg(target_os = "macos")]
fn symbol_font_paths() -> &'static [&'static str] {
    &["/System/Library/Fonts/Apple Symbols.ttf"]
}

#[cfg(target_os = "windows")]
fn symbol_font_paths() -> &'static [&'static str] {
    &[r"C:\Windows\Fonts\seguisym.ttf"]
}

#[cfg(target_os = "linux")]
fn symbol_font_paths() -> &'static [&'static str] {
    &[
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-symbols2-fonts/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-symbols-fonts/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-math-fonts/NotoSansMath-Regular.ttf",
        "/usr/local/share/NotoSansSymbols2-Regular.ttf",
        "/usr/local/share/NotoSansSymbols-Regular.ttf",
        "/usr/local/share/NotoSansMath-Regular.ttf",
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn symbol_font_paths() -> &'static [&'static str] {
    &[]
}

fn load_application_settings() -> ApplicationSettings {
    let Some(path) = configuration_file("application.json") else {
        return ApplicationSettings::default();
    };
    load_application_settings_path(&path)
}

fn load_application_settings_path(path: &Path) -> ApplicationSettings {
    match std::fs::read_to_string(path) {
        Ok(json) => ApplicationSettings::from_json(&json).unwrap_or_else(|error| {
            panic!(
                "invalid application configuration in {}: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => ApplicationSettings::default(),
        Err(error) => panic!(
            "cannot read application configuration from {}: {error}",
            path.display()
        ),
    }
}

fn load_input_bindings() -> input_bindings::InputBindings {
    let Some(path) = configuration_file("input_bindings.json") else {
        return default_input_bindings();
    };
    load_input_bindings_path(&path)
}

fn load_input_bindings_path(path: &Path) -> input_bindings::InputBindings {
    match std::fs::read_to_string(path) {
        Ok(json) => input_bindings::InputBindings::from_json(&json).unwrap_or_else(|error| {
            panic!("invalid input bindings in {}: {error}", path.display())
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => default_input_bindings(),
        Err(error) => panic!(
            "cannot read input bindings from {}: {error}",
            path.display()
        ),
    }
}

fn configuration_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join(APPLICATION_ID).join(name))
}

fn derived_cache_directory() -> PathBuf {
    application_cache_directory().join("derived")
}

fn application_cache_directory() -> PathBuf {
    std::cfg_select! {
        target_os = "macos" => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| application_directory(home.join("Library").join("Caches")))
            .unwrap_or_else(|| application_directory(std::env::temp_dir())),
        target_os = "windows" => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(application_directory)
            .unwrap_or_else(|| application_directory(std::env::temp_dir())),
        _ => std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".cache"))
            })
            .map(application_directory)
            .unwrap_or_else(|| application_directory(std::env::temp_dir())),
    }
}

fn application_directory(parent: PathBuf) -> PathBuf {
    parent.join(APPLICATION_ID)
}

struct NativeHostService {
    commands: crossbeam_channel::Receiver<HostCommand>,
}

struct NativeNodeFileDialogService;

impl FileDialogService for NativeNodeFileDialogService {
    fn available(&self, _save: bool) -> bool {
        true
    }

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        let mut dialog = rfd::FileDialog::new();
        if !request.title.is_empty() {
            dialog = dialog.set_title(request.title);
        }
        for filter in request.filters {
            let extensions = filter
                .extensions
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            dialog = dialog.add_filter(&filter.name, &extensions);
        }
        let selected = if request.save {
            dialog.save_file()
        } else {
            dialog.pick_file()
        };
        selected.map(|path| path.display().to_string())
    }
}

impl NativeHostService {
    fn new() -> Self {
        Self {
            commands: host_command_bridge().receiver.clone(),
        }
    }
}

impl HostService for NativeHostService {
    fn ui_capabilities(&self) -> HostUiCapabilities {
        #[cfg(target_os = "macos")]
        {
            HostUiCapabilities {
                direct_document_access: true,
                system_menu_bar: true,
                viewport_close_guard: false,
                modifier_key_labels: ModifierKeyLabels {
                    alternate: "Option",
                    command: "Command",
                },
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            HostUiCapabilities {
                direct_document_access: true,
                system_menu_bar: false,
                viewport_close_guard: true,
                modifier_key_labels: ModifierKeyLabels::default(),
            }
        }
    }

    fn decoded_block_cache_snapshot(&self) -> Option<DecodedBlockCacheSnapshot> {
        let stats = signal_processing::decoded_block_cache_stats();
        Some(DecodedBlockCacheSnapshot {
            entries: stats.entries,
            memory_bytes: stats.memory_bytes,
            budget_bytes: stats.budget_bytes,
            hits: stats.hits,
            misses: stats.misses,
        })
    }

    fn set_command_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        *host_command_bridge().repaint.lock().unwrap() = Some(repaint);
    }

    fn take_commands(&mut self) -> Vec<HostCommand> {
        self.commands.try_iter().collect()
    }

    fn publish_recent_files(&self, paths: &[PathBuf]) {
        #[cfg(target_os = "macos")]
        if let Some(listener) = RECENT_FILES_LISTENER.get() {
            listener(paths);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = paths;
    }

    fn document_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_file()
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .set_file_name(request.default_file_name)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.save_file()
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        serde_json::from_str(&json)
            .map_err(|error| format!("could not parse {}: {error}", path.display()))
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        let json = serde_json::to_string_pretty(graph)
            .map_err(|error| format!("could not serialize graph: {error}"))?;
        std::fs::write(path, json)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod native_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use logic_analyzer_graph_compiler::{
        PreparedCaptureData, SourcePreparationControl, SourcePreparationExecutor,
        SourcePreparationTaskUpdate,
    };
    use logic_analyzer_ui::{AppServices, HostCommand, HostService};
    use signal_processing::{
        AppManagerFactory, CooperativeWorkerOperationExecutor, InlineWorkExecutor,
        MemoryArtifactRepository, WorkExecutor, portable_worker_kernels,
    };

    use super::{
        NativeAppManagerFactory, NativeHostService, NativeRuntimeExecutor,
        NativeSourcePreparationExecutor, NativeWorkExecutor, application_directory,
        load_application_settings_path, load_input_bindings_path, queue_host_command,
    };

    #[test]
    fn native_runtime_factory_selects_the_threaded_backend() {
        let factory = NativeAppManagerFactory {
            work_executor: Arc::new(NativeRuntimeExecutor),
        };
        let mut manager = factory.create();

        manager.pump(1);
        assert!(manager.is_finished());
    }

    #[test]
    fn native_source_preparation_executor_completes_work_off_the_caller() {
        let executor = NativeSourcePreparationExecutor::new();
        let mut task = executor
            .submit(
                Box::new(|_| Ok(PreparedCaptureData::Channels(vec![(4, "Data".into())]))),
                SourcePreparationControl::new(),
            )
            .unwrap();

        for _ in 0..10_000 {
            match task.poll() {
                SourcePreparationTaskUpdate::Pending => std::thread::yield_now(),
                SourcePreparationTaskUpdate::Complete(Ok(PreparedCaptureData::Channels(
                    channels,
                ))) => {
                    assert_eq!(channels, vec![(4, "Data".into())]);
                    return;
                }
                SourcePreparationTaskUpdate::Complete(Ok(_)) => {
                    panic!("source preparation returned the wrong data kind");
                }
                SourcePreparationTaskUpdate::Complete(Err(error)) => {
                    panic!("source preparation failed: {error}");
                }
                SourcePreparationTaskUpdate::Disconnected => {
                    panic!("source preparation worker disconnected");
                }
            }
        }
        panic!("source preparation worker did not complete");
    }

    #[test]
    fn native_work_executor_runs_submitted_work() {
        let executor = NativeWorkExecutor::new();
        let (sender, receiver) = std::sync::mpsc::channel();

        executor
            .submit(Box::new(move || sender.send(42).unwrap()))
            .unwrap();

        assert!(executor.available_parallelism() >= 1);
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            42
        );
    }

    #[test]
    fn native_composition_preserves_injected_host_services_and_catalogs() {
        let services = crate::services::PlatformServices::with_ui_services(
            AppServices::with_host_service(Box::new(NativeHostService::new())),
            Vec::new(),
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(InlineWorkExecutor),
            std::rc::Rc::new(CooperativeWorkerOperationExecutor::new(
                portable_worker_kernels(),
                "test fallback",
            )),
        );
        assert_eq!(services.work_executor().available_parallelism(), 1);
        assert!(!services.artifact_repository().capabilities().durable);

        let (_, node_catalogs) = services.into_ui_and_node_catalogs();
        assert!(node_catalogs.is_empty());
    }

    #[test]
    fn native_shell_commands_wake_and_reach_the_ui_service_port() {
        let repaint_count = Arc::new(AtomicUsize::new(0));
        let callback_count = Arc::clone(&repaint_count);
        let mut host = NativeHostService::new();
        host.set_command_repaint(Box::new(move || {
            callback_count.fetch_add(1, Ordering::Relaxed);
        }));

        queue_host_command(HostCommand::Run);

        assert_eq!(repaint_count.load(Ordering::Relaxed), 1);
        assert_eq!(host.take_commands(), vec![HostCommand::Run]);
    }

    #[test]
    fn native_cache_directories_use_the_application_identifier() {
        let parent = tempfile::tempdir().unwrap();

        assert_eq!(
            application_directory(parent.path().to_owned()),
            parent.path().join("logic-conduit")
        );
    }

    #[test]
    fn native_configuration_files_override_embedded_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let application = directory.path().join("application.json");
        let input_bindings = directory.path().join("input_bindings.json");
        std::fs::write(
            &application,
            r#"{
                "logic_analyzer_viewer": { "color_profile": "classic" },
                "live_capture": { "max_recent_sessions": 7, "max_storage_gib": 12 }
            }"#,
        )
        .unwrap();
        std::fs::write(
            &input_bindings,
            r#"{"bindings":[
                {"context":"custom","action":"only","label":"Only","input":"key","key":"f12"}
            ]}"#,
        )
        .unwrap();

        let settings = load_application_settings_path(&application);
        let bindings = load_input_bindings_path(&input_bindings);

        assert_eq!(settings.max_recent_capture_sessions(), 7);
        assert_eq!(settings.max_capture_storage_gib(), 12);
        assert!(bindings.shortcut(&["custom"], "only").is_some());
        assert!(bindings.shortcut(&["global"], "save").is_none());
    }

    #[test]
    fn missing_native_configuration_files_use_embedded_defaults() {
        let directory = tempfile::tempdir().unwrap();

        let settings = load_application_settings_path(&directory.path().join("missing.json"));
        let bindings = load_input_bindings_path(&directory.path().join("missing.json"));

        assert_eq!(settings.max_recent_capture_sessions(), 10);
        assert_eq!(settings.max_capture_storage_gib(), 20);
        assert!(bindings.shortcut(&["global"], "save").is_some());
    }
}
