use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use js_sys::Uint8Array;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

use logic_analyzer_capture_formats::CaptureSourceConstructionError;
use logic_analyzer_capture_formats::dsl_file::{
    DslFileSource, DslFileSourceConfig, DslFileSourceFactory,
};
use logic_analyzer_capture_formats::sigrok_file::{
    SigrokFileSource, SigrokFileSourceConfig, SigrokFileSourceFactory, portable_source_factory,
};
use platform_artifacts::{
    ArtifactRepository, ByteRange, PreparedByteSource, RandomAccessReader, SourceCapabilities,
    SourceIdentity, SourceReadError,
};
use platform_runtime::{WorkExecutor, WorkerOperation};
use signal_capture::{
    CaptureIndexPreparationRequest, CaptureMetadata, CaptureWorkerOperationRegistry,
    CaptureWorkerPreparedIndex,
};
use signal_capture_session::{
    CaptureSourceCacheIdentity, CaptureSourceKind, CaptureSourceLifecycle, CaptureSourceMetadata,
    CaptureSourceMetadataError, CaptureSourcePresentation,
};
use signal_runtime::{ProcessNode, ProcessNodeConstruction};

const DSL_PREPARATION_OPERATION: &str = "logic-analyzer.dsl-file.prepare/v1";
const SIGROK_PREPARATION_OPERATION: &str = "logic-analyzer.sigrok-file.prepare/v1";
const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;
const FILE_SOURCE_LIFECYCLE: CaptureSourceLifecycle =
    CaptureSourceLifecycle::new(CaptureSourceKind::File, true, true, true);

thread_local! {
    static WORKER_CAPTURES: RefCell<HashMap<String, WorkerCapture>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub(crate) struct WorkerCaptureReference {
    reference: String,
    display_name: String,
    identity: SourceIdentity,
    length: u64,
}

impl WorkerCaptureReference {
    pub(crate) fn new(
        reference: String,
        display_name: String,
        identity: SourceIdentity,
        length: u64,
    ) -> Self {
        Self {
            reference,
            display_name,
            identity,
            length,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct WorkerCapturePayload {
    reference: String,
    display_name: String,
    identity: SourceIdentity,
    length: u64,
}

#[derive(Clone)]
struct WorkerCapture {
    source: Arc<dyn PreparedByteSource>,
    display_name: String,
    identity: SourceIdentity,
    metadata: CaptureMetadata,
}

struct BrowserWorkerByteSource {
    reference: String,
    identity: SourceIdentity,
    length: u64,
}

struct BrowserWorkerByteReader {
    reference: String,
    length: u64,
}

impl PreparedByteSource for BrowserWorkerByteSource {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError> {
        Ok(Box::new(BrowserWorkerByteReader {
            reference: self.reference.clone(),
            length: self.length,
        }))
    }
}

impl RandomAccessReader for BrowserWorkerByteReader {
    fn len(&self) -> Result<u64, SourceReadError> {
        Ok(self.length)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        let requested =
            u64::try_from(destination.len()).map_err(|_| SourceReadError::RangeOverflow {
                offset,
                length: u64::MAX,
            })?;
        let range = ByteRange::new(offset, requested)?;
        if offset >= self.length {
            return Ok(0);
        }
        let available = range.end().min(self.length) - offset;
        if offset > MAX_SAFE_INTEGER || available > MAX_SAFE_INTEGER {
            return Err(SourceReadError::Io(
                "browser file range exceeds JavaScript's exact integer range".to_owned(),
            ));
        }
        let bytes = read_capture_file_range(&self.reference, offset as f64, available as f64)
            .map_err(|error| SourceReadError::Io(js_error(error)))?;
        let count = usize::try_from(bytes.length()).map_err(|_| {
            SourceReadError::Io("browser file range exceeds this address space".to_owned())
        })?;
        if count > destination.len() {
            return Err(SourceReadError::Io(
                "browser file reader returned more bytes than requested".to_owned(),
            ));
        }
        bytes.copy_to(&mut destination[..count]);
        Ok(count)
    }
}

pub(crate) fn dsl_preparation_request(
    reference: &WorkerCaptureReference,
) -> CaptureIndexPreparationRequest {
    preparation_request(DSL_PREPARATION_OPERATION, reference)
}

pub(crate) fn sigrok_preparation_request(
    reference: &WorkerCaptureReference,
) -> CaptureIndexPreparationRequest {
    preparation_request(SIGROK_PREPARATION_OPERATION, reference)
}

pub(crate) fn capture_worker_operations() -> CaptureWorkerOperationRegistry {
    let mut operations = CaptureWorkerOperationRegistry::new();
    operations
        .register(
            WorkerOperation::new(DSL_PREPARATION_OPERATION)
                .expect("the DSL capture-worker operation is valid"),
            |payload| {
                let (source, display_name, identity) = decode_source(payload)?;
                let presentation =
                    DslFileSource::indexed_capture_presentation(source, display_name);
                Ok(CaptureWorkerPreparedIndex::new(
                    identity,
                    presentation.factory,
                ))
            },
        )
        .expect("the DSL capture-worker operation is registered once");
    operations
        .register(
            WorkerOperation::new(SIGROK_PREPARATION_OPERATION)
                .expect("the Sigrok capture-worker operation is valid"),
            |payload| {
                let (source, display_name, identity) = decode_source(payload)?;
                let presentation =
                    SigrokFileSource::indexed_capture_presentation(source, display_name);
                Ok(CaptureWorkerPreparedIndex::new(
                    identity,
                    presentation.factory,
                ))
            },
        )
        .expect("the Sigrok capture-worker operation is registered once");
    operations
}

pub(crate) fn capture_metadata(
    reference: String,
    display_name: String,
    identity: SourceIdentity,
    length: u64,
) -> Result<CaptureMetadata, String> {
    let source = byte_source(reference.clone(), identity, length)?;
    let factory = if display_name.to_ascii_lowercase().ends_with(".sr") {
        SigrokFileSource::indexed_capture_presentation(Arc::clone(&source), display_name.clone())
            .factory
    } else {
        DslFileSource::indexed_capture_presentation(Arc::clone(&source), display_name.clone())
            .factory
    };
    let metadata = factory.metadata().map_err(|error| error.to_string())?;
    WORKER_CAPTURES.with(|captures| {
        captures.borrow_mut().insert(
            reference,
            WorkerCapture {
                source,
                display_name,
                identity,
                metadata: metadata.clone(),
            },
        );
    });
    Ok(metadata)
}

pub(crate) fn worker_dsl_file_source_factory() -> Arc<dyn DslFileSourceFactory> {
    Arc::new(WorkerDslFileSourceFactory)
}

pub(crate) fn worker_sigrok_file_source_factory() -> Arc<dyn SigrokFileSourceFactory> {
    Arc::new(WorkerSigrokFileSourceFactory)
}

struct WorkerDslFileSourceFactory;

struct WorkerDslFileSourceMetadata {
    config: DslFileSourceConfig,
}

impl CaptureSourceMetadata for WorkerDslFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let capture = worker_capture(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            DslFileSource::indexed_capture_presentation(capture.source, capture.display_name),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        worker_capture(self.config.path())
            .map(|capture| CaptureSourceCacheIdentity::Stable(*capture.identity.as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        worker_capture(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)
            .map(|capture| Some(capture.metadata.probe_names))
    }
}

impl DslFileSourceFactory for WorkerDslFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: DslFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(WorkerDslFileSourceMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: DslFileSourceConfig,
        artifact_repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    > {
        let metadata = self.metadata(config.clone());
        let capture =
            worker_capture(config.path()).map_err(CaptureSourceConstructionError::diagnostic)?;
        let source = DslFileSource::from_prepared_source(capture.source, capture.display_name)
            .map_err(CaptureSourceConstructionError::from)?
            .with_name(name)
            .with_artifact_repository(artifact_repository)
            .with_work_executor(work_executor);
        Ok(ProcessNodeConstruction::new(Box::new(source), metadata))
    }
}

struct WorkerSigrokFileSourceFactory;

struct WorkerSigrokFileSourceMetadata {
    config: SigrokFileSourceConfig,
}

impl CaptureSourceMetadata for WorkerSigrokFileSourceMetadata {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn presentation(
        &self,
    ) -> Result<Option<CaptureSourcePresentation>, CaptureSourceMetadataError> {
        if self.config.demo_data() {
            return portable_source_factory()
                .metadata(self.config.clone())
                .presentation();
        }
        if self.config.path().as_os_str().is_empty() {
            return Ok(None);
        }
        let capture = worker_capture(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)?;
        Ok(Some(CaptureSourcePresentation::Indexed(
            SigrokFileSource::indexed_capture_presentation(capture.source, capture.display_name),
        )))
    }

    fn cache_identity(&self) -> CaptureSourceCacheIdentity {
        if self.config.demo_data() {
            return CaptureSourceCacheIdentity::NotCapture;
        }
        worker_capture(self.config.path())
            .map(|capture| CaptureSourceCacheIdentity::Stable(*capture.identity.as_bytes()))
            .unwrap_or(CaptureSourceCacheIdentity::Dynamic)
    }

    fn channel_names(&self) -> Result<Option<Vec<String>>, CaptureSourceMetadataError> {
        if self.config.demo_data() {
            return Ok(Some(self.config.channel_names().to_vec()));
        }
        worker_capture(self.config.path())
            .map_err(CaptureSourceMetadataError::access_message)
            .map(|capture| Some(capture.metadata.probe_names))
    }
}

impl SigrokFileSourceFactory for WorkerSigrokFileSourceFactory {
    fn lifecycle(&self) -> CaptureSourceLifecycle {
        FILE_SOURCE_LIFECYCLE
    }

    fn metadata(&self, config: SigrokFileSourceConfig) -> Arc<dyn CaptureSourceMetadata> {
        Arc::new(WorkerSigrokFileSourceMetadata { config })
    }

    fn create(
        &self,
        name: &str,
        config: SigrokFileSourceConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<
        ProcessNodeConstruction<Arc<dyn CaptureSourceMetadata>>,
        CaptureSourceConstructionError,
    > {
        if config.demo_data() {
            return portable_source_factory().create(name, config, work_executor);
        }
        let metadata = self.metadata(config.clone());
        let capture =
            worker_capture(config.path()).map_err(CaptureSourceConstructionError::diagnostic)?;
        let source = SigrokFileSource::from_prepared_source(capture.source)
            .map_err(CaptureSourceConstructionError::from)?
            .with_name(name)
            .with_work_executor(work_executor);
        Ok(ProcessNodeConstruction::new(
            Box::new(source) as Box<dyn ProcessNode>,
            metadata,
        ))
    }
}

fn worker_capture(path: &std::path::Path) -> Result<WorkerCapture, String> {
    let reference = path.to_string_lossy();
    WORKER_CAPTURES.with(|captures| {
        captures
            .borrow()
            .get(reference.as_ref() as &str)
            .cloned()
            .ok_or_else(|| {
                format!("browser capture '{reference}' is not attached to the graph worker")
            })
    })
}

fn preparation_request(
    operation: &str,
    reference: &WorkerCaptureReference,
) -> CaptureIndexPreparationRequest {
    let payload = WorkerCapturePayload {
        reference: reference.reference.clone(),
        display_name: reference.display_name.clone(),
        identity: reference.identity,
        length: reference.length,
    };
    CaptureIndexPreparationRequest::new(
        WorkerOperation::new(operation).expect("capture-worker operation identifiers are valid"),
        serde_json::to_vec(&payload).expect("the browser capture reference is serializable"),
    )
}

fn decode_source(
    payload: Vec<u8>,
) -> Result<(Arc<dyn PreparedByteSource>, String, SourceIdentity), String> {
    let payload = serde_json::from_slice::<WorkerCapturePayload>(&payload)
        .map_err(|error| format!("invalid browser capture reference: {error}"))?;
    let source = byte_source(payload.reference, payload.identity, payload.length)?;
    Ok((source, payload.display_name, payload.identity))
}

fn byte_source(
    reference: String,
    identity: SourceIdentity,
    length: u64,
) -> Result<Arc<dyn PreparedByteSource>, String> {
    if length > MAX_SAFE_INTEGER {
        return Err("browser capture length exceeds JavaScript's exact integer range".to_owned());
    }
    Ok(Arc::new(BrowserWorkerByteSource {
        reference,
        identity,
        length,
    }))
}

fn js_error(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(
        catch,
        js_namespace = globalThis,
        js_name = logicConduitReadCaptureRange
    )]
    fn read_capture_file_range(
        reference: &str,
        offset: f64,
        length: f64,
    ) -> Result<Uint8Array, JsValue>;
}
