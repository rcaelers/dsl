use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rusb::{Context, DeviceHandle, UsbContext};

use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogScanner, SigrokCatalogSnapshot, SigrokDecoder, SigrokDecoderConfig,
    SigrokDecoderDescriptor, SigrokDecoderRuntime,
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
use signal_artifacts::{ArtifactRepository, PreparedByteSource, SourceIdentity};
use signal_capture::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, IndexedCapturePresentation,
};
use signal_capture_session::logic_analyzer::LogicAnalyzerError;
use signal_runtime::{
    AppManager, AppManagerBackend, AppManagerFactory, PipelineManager, ProcessNode, WorkExecutor,
    WorkExecutorTask, WorkTask,
};

use super::native_artifact_repository::NativeArtifactRepository;
use super::native_file_identity_cache::NativeFileIdentityCache;
use super::native_file_source::NativeFileByteSource;
use super::native_sigrok::{PythonSigrokExecutionFactory, discover_sigrok_decoder, scan_catalog};
use super::native_worker::NativeWorkerOperationExecutor;
use crate::services::PlatformServices;

pub(crate) fn standard_services(application_id: &str) -> PlatformServices {
    let cache_directory = derived_cache_directory(application_id);
    let artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository> = Arc::new(
        NativeArtifactRepository::new(cache_directory.join("artifacts")),
    );
    let work_executor: Arc<dyn WorkExecutor> = Arc::new(NativeWorkExecutor::new());
    let sigrok_catalog_scanner = native_sigrok_catalog_scanner();
    let dsl_file_source_factory = native_dsl_file_source_factory();
    let sigrok_file_source_factory = native_sigrok_file_source_factory();
    PlatformServices {
        capture_worker_client: None,
        app_manager_factory: Arc::new(NativeAppManagerFactory {
            work_executor: Arc::new(NativeRuntimeExecutor),
        }),
        dsl_file_source_factory,
        sigrok_file_source_factory,
        sigrok_decoder_runtime: Some(native_sigrok_decoder_runtime()),
        sigrok_catalog_scanner: Some(sigrok_catalog_scanner),
        u3pro16_source_factory: Some(native_u3pro16_source_factory()),
        output_storage: Some(native_output_storage()),
        file_picker: None,
        artifact_repository,
        work_executor,
        worker_operation_executor: Rc::new(NativeWorkerOperationExecutor::new()),
        graph_worker_client: None,
    }
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

    fn metadata(&self) -> signal_capture::Result<signal_capture::CaptureMetadata> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_capture::Error::ParseError)?;
        DslFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .metadata()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_capture::Error::ParseError)?;
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

    fn metadata(&self) -> signal_capture::Result<signal_capture::CaptureMetadata> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_capture::Error::ParseError)?;
        SigrokFileSource::indexed_capture_presentation(source, self.path.display().to_string())
            .factory
            .metadata()
    }

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> signal_capture::Result<Box<dyn CaptureIndex + Send>> {
        let source = acquire_native_file(&self.path, &self.identities)
            .map_err(signal_capture::Error::ParseError)?;
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
        artifact_repository: Arc<dyn signal_artifacts::ArtifactRepository>,
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

const U3PRO16_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::Live, false, true, true);

struct NativeU3Pro16SourceFactory;

impl DsLogicU3Pro16SourceFactory for NativeU3Pro16SourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        U3PRO16_LIFECYCLE
    }

    fn metadata(
        &self,
        config: signal_capture_session::logic_analyzer::LogicCaptureConfig,
    ) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(NativeU3Pro16Metadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: signal_capture_session::logic_analyzer::LogicCaptureConfig,
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
    config: signal_capture_session::logic_analyzer::LogicCaptureConfig,
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
    ) -> Result<Option<Box<dyn signal_capture_session::ConfiguredAcquisition>>, String> {
        let channels = self
            .enabled_channels()
            .map(|channel| {
                signal_capture_session::CaptureChannelId::new(format!("u3pro16:input:{channel}"))
            })
            .collect::<Vec<_>>();
        DsLogicU3Pro16Capture::new(
            self.config.clone(),
            channels,
            native_u3pro16_transport_factory(),
        )
        .map(|capture| {
            Some(Box::new(capture) as Box<dyn signal_capture_session::ConfiguredAcquisition>)
        })
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
    ) -> signal_capture_session::logic_analyzer::LogicAnalyzerResult<Box<dyn UsbTransport>> {
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
    fn open_first() -> signal_capture_session::logic_analyzer::LogicAnalyzerResult<Self> {
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
    ) -> signal_capture_session::logic_analyzer::LogicAnalyzerResult<Option<Vec<u8>>> {
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

    fn add_node(&mut self, spec: signal_runtime::NodeSpec) -> Result<(), String> {
        self.manager.add_node(spec)
    }

    fn add_node_deferred(&mut self, spec: signal_runtime::NodeSpec) -> Result<(), String> {
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
        config: signal_runtime::NodeConfig,
    ) -> Result<(), String> {
        self.manager.reconfigure(name, config)
    }

    fn reconfigure_at(
        &mut self,
        name: &str,
        config: signal_runtime::NodeConfig,
        boundary: signal_runtime::ConfigurationBoundary,
    ) -> Result<(), String> {
        self.manager.reconfigure_at(name, config, boundary)
    }

    fn restart_node(
        &mut self,
        name: &str,
        node: Box<dyn signal_runtime::ProcessNode>,
        inputs: Vec<Option<signal_runtime::InputSub>>,
    ) -> Result<(), String> {
        self.manager.restart_node(name, node, inputs)
    }

    fn progress(&self) -> Vec<(String, u64)> {
        self.manager.progress()
    }

    fn take_disconnected(&self) -> Vec<signal_runtime::DisconnectEvent> {
        self.manager.take_disconnected()
    }

    fn take_failures(&mut self) -> Vec<signal_runtime::NodeFailure> {
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

fn derived_cache_directory(application_id: &str) -> PathBuf {
    application_cache_directory(application_id).join("derived")
}

fn application_cache_directory(application_id: &str) -> PathBuf {
    std::cfg_select! {
        target_os = "macos" => std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| application_directory(home.join("Library").join("Caches"), application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
        target_os = "windows" => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|directory| application_directory(directory, application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
        _ => std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".cache"))
            })
            .map(|directory| application_directory(directory, application_id))
            .unwrap_or_else(|| application_directory(std::env::temp_dir(), application_id)),
    }
}

fn application_directory(parent: PathBuf, application_id: &str) -> PathBuf {
    parent.join(application_id)
}

#[cfg(test)]
mod native_tests {
    use std::sync::Arc;

    use signal_runtime::{AppManagerFactory, WorkExecutor};

    use super::{
        NativeAppManagerFactory, NativeRuntimeExecutor, NativeWorkExecutor, application_directory,
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
    fn native_cache_directories_use_the_application_identifier() {
        let parent = tempfile::tempdir().unwrap();

        assert_eq!(
            application_directory(parent.path().to_owned(), "logic-conduit"),
            parent.path().join("logic-conduit")
        );
    }
}
