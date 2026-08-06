use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::{Arc, Once};

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker, WorkerOptions, WorkerType,
};

use logic_analyzer_graph_orchestration::{
    GraphWorkerClient, GraphWorkerRequest, GraphWorkerRuntime, decode_graph_worker_messages,
    decode_graph_worker_request, encode_graph_worker_messages, encode_graph_worker_request,
};
use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository, SourceIdentity};
use platform_runtime::InlineWorkExecutor;
use signal_capture::{
    CaptureMetadata, CaptureWorkerClient, CaptureWorkerRequest, CaptureWorkerRuntime,
    decode_capture_worker_messages, encode_capture_worker_messages,
};

const WORKER_BOOTSTRAP: &str = include_str!("capture_worker_bootstrap.js");
const PUMP_INTERVAL_MS: i32 = 4;
const WORKER_PANIC_PROPERTY: &str = "logicConduitWorkerPanic";

thread_local! {
    static CAPTURE_WORKER_RUNTIMES: RefCell<Vec<BrowserCaptureWorkerRuntime>> = const { RefCell::new(Vec::new()) };
    static WORKER_ARTIFACT_REPOSITORY: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
    static CAPTURE_WORKER_RUNTIME: RefCell<CaptureWorkerRuntime> = RefCell::new(CaptureWorkerRuntime::new(
        super::web_file_import::capture_worker_operations(),
        WORKER_ARTIFACT_REPOSITORY.with(Arc::clone),
        Arc::new(InlineWorkExecutor),
    ));
    static GRAPH_WORKER_RUNTIME: RefCell<Option<GraphWorkerRuntime>> = const { RefCell::new(None) };
    static CAPTURE_IDENTITY_HASHERS: RefCell<CaptureIdentityHashers> = RefCell::new(CaptureIdentityHashers::default());
}

/// Installs the application-owned graph runtime in this worker instance.
pub fn initialize_graph_worker_runtime(graph_worker_runtime: GraphWorkerRuntime) {
    GRAPH_WORKER_RUNTIME.with(|runtime| {
        *runtime.borrow_mut() = Some(graph_worker_runtime);
    });
}

/// Returns the artifact repository retained by capture and graph runtimes in this worker.
pub fn worker_artifact_repository() -> Arc<dyn ArtifactRepository> {
    WORKER_ARTIFACT_REPOSITORY.with(Arc::clone)
}

type AttachmentProgress = Box<dyn Fn(u64, u64)>;
type AttachmentComplete = Box<dyn FnOnce(Result<AttachedCapture, String>)>;

pub(crate) struct AttachedCapture {
    pub(crate) identity: SourceIdentity,
    pub(crate) metadata: CaptureMetadata,
}

pub(crate) struct BrowserWorkerClients {
    pub(crate) capture: Arc<CaptureWorkerClient>,
    pub(crate) graph: Arc<GraphWorkerClient>,
}

struct AttachmentCallbacks {
    progress: AttachmentProgress,
    complete: AttachmentComplete,
}

#[derive(Default)]
struct CaptureIdentityHashers {
    next_id: u32,
    hashers: BTreeMap<u32, blake3::Hasher>,
}

struct BrowserCaptureWorkerRuntime {
    worker: Worker,
    _message_handler: Closure<dyn FnMut(MessageEvent)>,
    _error_handler: Closure<dyn FnMut(ErrorEvent)>,
    _pump: Closure<dyn FnMut()>,
    interval_id: i32,
    attachments: Rc<RefCell<HashMap<String, AttachmentCallbacks>>>,
}

impl Drop for BrowserCaptureWorkerRuntime {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_interval_with_handle(self.interval_id);
        }
        self.worker.terminate();
    }
}

pub(crate) fn install_capture_worker(
    module_url: &str,
    wasm_url: &str,
    max_outstanding: usize,
    artifact_repository: Arc<dyn ArtifactRepository>,
) -> Result<BrowserWorkerClients, String> {
    let capture_client = Arc::new(CaptureWorkerClient::new(max_outstanding)?);
    let graph_client = Arc::new(GraphWorkerClient::new(1, artifact_repository)?);
    let worker_url = create_worker_url()?;
    let worker = match create_worker(&worker_url) {
        Ok(worker) => worker,
        Err(error) => {
            let _ = Url::revoke_object_url(&worker_url);
            return Err(error);
        }
    };
    if let Err(error) = Url::revoke_object_url(&worker_url) {
        worker.terminate();
        return Err(js_error(
            "could not release capture-worker bootstrap URL",
            error,
        ));
    }

    let ready = Rc::new(Cell::new(false));
    let disconnected = Rc::new(Cell::new(false));
    let attachments = Rc::new(RefCell::new(HashMap::new()));
    let message_capture_client = Arc::clone(&capture_client);
    let message_graph_client = Arc::clone(&graph_client);
    let message_ready = Rc::clone(&ready);
    let message_disconnected = Rc::clone(&disconnected);
    let message_attachments = Rc::clone(&attachments);
    let message_handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        handle_worker_message(
            &message_capture_client,
            &message_graph_client,
            &message_ready,
            &message_disconnected,
            &message_attachments,
            event.data(),
        );
    });
    let error_capture_client = Arc::clone(&capture_client);
    let error_graph_client = Arc::clone(&graph_client);
    let error_disconnected = Rc::clone(&disconnected);
    let error_attachments = Rc::clone(&attachments);
    let error_handler = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        event.prevent_default();
        disconnect(
            &error_capture_client,
            &error_graph_client,
            &error_disconnected,
            format!("capture worker failed: {}", event.message()),
        );
        fail_attachments(
            &error_attachments,
            format!("capture worker failed: {}", event.message()),
        );
    });
    worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));

    if let Err(error) = post_initialize(&worker, module_url, wasm_url) {
        worker.terminate();
        return Err(error);
    }

    let pump_worker = worker.clone();
    let pump_capture_client = Arc::clone(&capture_client);
    let pump_graph_client = Arc::clone(&graph_client);
    let pump_ready = Rc::clone(&ready);
    let pump_disconnected = Rc::clone(&disconnected);
    let pump = Closure::<dyn FnMut()>::new(move || {
        if !pump_ready.get() || pump_disconnected.get() {
            return;
        }
        for request in pump_capture_client.drain_requests() {
            if let Err(error) = post_capture_request(&pump_worker, &request) {
                disconnect(
                    &pump_capture_client,
                    &pump_graph_client,
                    &pump_disconnected,
                    error,
                );
                break;
            }
        }
        for request in pump_graph_client.drain_requests() {
            if let Err(error) = post_graph_request(&pump_worker, &request) {
                disconnect(
                    &pump_capture_client,
                    &pump_graph_client,
                    &pump_disconnected,
                    error,
                );
                break;
            }
        }
    });
    let window = web_sys::window().ok_or_else(|| "browser window is unavailable".to_owned())?;
    let interval_id = window
        .set_interval_with_callback_and_timeout_and_arguments_0(
            pump.as_ref().unchecked_ref(),
            PUMP_INTERVAL_MS,
        )
        .map_err(|error| js_error("could not start capture-worker pump", error))?;

    CAPTURE_WORKER_RUNTIMES.with(|runtimes| {
        runtimes.borrow_mut().push(BrowserCaptureWorkerRuntime {
            worker,
            _message_handler: message_handler,
            _error_handler: error_handler,
            _pump: pump,
            interval_id,
            attachments,
        });
    });
    Ok(BrowserWorkerClients {
        capture: capture_client,
        graph: graph_client,
    })
}

pub(crate) fn attach_capture_file(
    reference: &str,
    display_name: &str,
    file: &web_sys::File,
    progress: AttachmentProgress,
    complete: AttachmentComplete,
) -> Result<bool, String> {
    CAPTURE_WORKER_RUNTIMES.with(|runtimes| {
        let runtimes = runtimes.borrow();
        let Some(runtime) = runtimes.last() else {
            return Ok(false);
        };
        let message = message_object("capture_attach")?;
        set(&message, "reference", JsValue::from_str(reference))?;
        set(&message, "displayName", JsValue::from_str(display_name))?;
        set(&message, "file", file.clone().into())?;
        runtime.attachments.borrow_mut().insert(
            reference.to_owned(),
            AttachmentCallbacks { progress, complete },
        );
        if let Err(error) = runtime
            .worker
            .post_message(&message)
            .map_err(|error| js_error("could not attach browser file to capture worker", error))
        {
            runtime.attachments.borrow_mut().remove(reference);
            return Err(error);
        }
        Ok(true)
    })
}

pub(crate) fn cancel_capture_file_attachment(reference: &str) {
    CAPTURE_WORKER_RUNTIMES.with(|runtimes| {
        let runtimes = runtimes.borrow();
        let Some(runtime) = runtimes.last() else {
            return;
        };
        runtime.attachments.borrow_mut().remove(reference);
        if let Ok(message) = message_object("capture_detach") {
            let _ = set(&message, "reference", JsValue::from_str(reference));
            let _ = runtime.worker.post_message(&message);
        }
    });
}

fn handle_worker_message(
    capture_client: &Arc<CaptureWorkerClient>,
    graph_client: &Arc<GraphWorkerClient>,
    ready: &Rc<Cell<bool>>,
    disconnected: &Rc<Cell<bool>>,
    attachments: &Rc<RefCell<HashMap<String, AttachmentCallbacks>>>,
    value: JsValue,
) {
    let kind = string_property(&value, "kind").unwrap_or_default();
    match kind.as_str() {
        "ready" => ready.set(true),
        "capture_messages" => {
            let result = property(&value, "payload")
                .map(|payload| Uint8Array::new(&payload).to_vec())
                .and_then(|payload| {
                    decode_capture_worker_messages(&payload).map_err(|error| {
                        format!("capture worker returned an invalid message: {error}")
                    })
                })
                .and_then(|messages| {
                    for message in messages {
                        capture_client.publish(message)?;
                    }
                    Ok(())
                });
            if let Err(error) = result {
                disconnect(capture_client, graph_client, disconnected, error);
            }
        }
        "graph_messages" => {
            let result = property(&value, "payload")
                .map(|payload| Uint8Array::new(&payload).to_vec())
                .and_then(|payload| {
                    decode_graph_worker_messages(&payload).map_err(|error| {
                        format!("graph worker returned an invalid message: {error}")
                    })
                })
                .and_then(|messages| {
                    for message in messages {
                        graph_client.publish(message)?;
                    }
                    Ok(())
                });
            if let Err(error) = result {
                disconnect(capture_client, graph_client, disconnected, error);
            }
        }
        "graph_output_files" => {
            let result = property(&value, "payload")
                .map(|payload| Uint8Array::new(&payload).to_vec())
                .and_then(|payload| {
                    serde_json::from_slice::<Vec<super::web_output_storage::BrowserOutputFile>>(
                        &payload,
                    )
                    .map_err(|error| format!("graph worker returned invalid output files: {error}"))
                })
                .map(|files| {
                    files
                        .into_iter()
                        .map(|file| logic_analyzer_platform::BrowserDownloadFile {
                            name: file.name,
                            bytes: file.bytes,
                            annotations: vec![file.producer_node, file.producer_socket],
                        })
                        .collect::<Vec<_>>()
                })
                .map(logic_analyzer_platform::queue_browser_downloads);
            if let Err(error) = result {
                tracing::warn!(%error, "browser graph output download failed");
            }
        }
        "capture_attach_progress" => {
            if let (Ok(reference), Ok(completed), Ok(total)) = (
                string_property(&value, "reference"),
                integer_property(&value, "completed"),
                integer_property(&value, "total"),
            ) && let Some(callbacks) = attachments.borrow().get(&reference)
            {
                (callbacks.progress)(completed, total);
            }
        }
        "capture_attached" => {
            let result = decode_attached_capture(&value);
            if let Ok(reference) = string_property(&value, "reference")
                && let Some(callbacks) = attachments.borrow_mut().remove(&reference)
            {
                (callbacks.complete)(result);
            }
        }
        "capture_attach_failed" => {
            if let Ok(reference) = string_property(&value, "reference")
                && let Some(callbacks) = attachments.borrow_mut().remove(&reference)
            {
                let message = string_property(&value, "message")
                    .unwrap_or_else(|error| format!("browser capture import failed: {error}"));
                (callbacks.complete)(Err(message));
            }
        }
        "worker_failed" => {
            let message = string_property(&value, "message")
                .unwrap_or_else(|error| format!("capture worker failed: {error}"));
            disconnect(capture_client, graph_client, disconnected, message);
        }
        _ => disconnect(
            capture_client,
            graph_client,
            disconnected,
            "capture worker returned an unknown message".to_owned(),
        ),
    }
}

fn decode_attached_capture(value: &JsValue) -> Result<AttachedCapture, String> {
    let identity = Uint8Array::new(&property(value, "identity")?).to_vec();
    let identity: [u8; 32] = identity
        .try_into()
        .map_err(|_| "capture worker returned an invalid content identity".to_owned())?;
    let metadata = Uint8Array::new(&property(value, "metadata")?).to_vec();
    let metadata = serde_json::from_slice(&metadata)
        .map_err(|error| format!("capture worker returned invalid metadata: {error}"))?;
    Ok(AttachedCapture {
        identity: SourceIdentity::from_bytes(identity),
        metadata,
    })
}

fn fail_attachments(
    attachments: &Rc<RefCell<HashMap<String, AttachmentCallbacks>>>,
    message: String,
) {
    for (_, callbacks) in attachments.borrow_mut().drain() {
        (callbacks.complete)(Err(message.clone()));
    }
}

#[wasm_bindgen(js_name = executeCaptureWorkerRequest)]
/// Executes one serialized capture-worker request in the browser worker.
///
/// Messages produced while handling the request are passed to `publish` as encoded
/// capture-worker message batches. The returned boolean reports whether preparation
/// work remains and must be advanced with [`advance_capture_worker_preparation`].
///
/// # Parameters
/// - `payload`: JSON-encoded [`CaptureWorkerRequest`].
/// - `publish`: JavaScript callback receiving each encoded message batch.
pub fn execute_capture_worker_request(
    payload: Vec<u8>,
    publish: &Function,
) -> Result<bool, JsValue> {
    install_worker_panic_hook();
    let request = serde_json::from_slice::<CaptureWorkerRequest>(&payload)
        .map_err(|error| JsValue::from_str(&format!("invalid capture-worker request: {error}")))?;
    let mut failure = None;
    CAPTURE_WORKER_RUNTIME.with(|runtime| {
        runtime
            .borrow_mut()
            .execute_streaming(request, &mut |message| {
                publish_capture_message(publish, message, &mut failure);
            });
    });
    failure.map_or_else(
        || Ok(CAPTURE_WORKER_RUNTIME.with(|runtime| runtime.borrow().has_pending_preparations())),
        Err,
    )
}

#[wasm_bindgen(js_name = advanceCaptureWorkerPreparation)]
/// Advances one pending capture preparation in the browser worker.
///
/// Messages produced during the step are passed to `publish`; the returned boolean
/// indicates whether any preparation remains pending.
///
/// # Parameters
/// - `publish`: JavaScript callback receiving each encoded message batch.
pub fn advance_capture_worker_preparation(publish: &Function) -> Result<bool, JsValue> {
    install_worker_panic_hook();
    let mut failure = None;
    let pending = CAPTURE_WORKER_RUNTIME.with(|runtime| {
        runtime.borrow_mut().advance_streaming(&mut |message| {
            publish_capture_message(publish, message, &mut failure);
        })
    });
    failure.map_or(Ok(pending), Err)
}

#[wasm_bindgen(js_name = executeGraphWorkerRequest)]
/// Executes one serialized graph-worker request in the browser worker.
///
/// Messages produced while handling the request are passed to `publish` as encoded
/// graph-worker message batches. The returned boolean reports whether a graph run
/// remains active and must be advanced with [`advance_graph_worker_run`].
///
/// # Parameters
/// - `payload`: Binary-encoded [`GraphWorkerRequest`].
/// - `publish`: JavaScript callback receiving each encoded message batch.
pub fn execute_graph_worker_request(payload: Vec<u8>, publish: &Function) -> Result<bool, JsValue> {
    install_worker_panic_hook();
    let request = decode_graph_worker_request(&payload)
        .map_err(|error| JsValue::from_str(&format!("invalid graph-worker request: {error}")))?;
    if matches!(&request, GraphWorkerRequest::Start { .. }) {
        super::web_output_storage::begin_output_run();
    }
    let mut failure = None;
    let active = GRAPH_WORKER_RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let runtime = runtime.as_mut().ok_or_else(|| {
            JsValue::from_str("graph-worker runtime was not initialized by the application")
        })?;
        runtime.execute_streaming(request, &mut |message| {
            publish_graph_message(publish, message, &mut failure);
        });
        Ok::<bool, JsValue>(runtime.has_active_run())
    })?;
    failure.map_or(Ok(active), Err)
}

#[wasm_bindgen(js_name = advanceGraphWorkerRun)]
/// Advances the active graph run by one cooperative browser-worker step.
///
/// Messages produced during the step are passed to `publish`; the returned boolean
/// indicates whether the graph run remains active.
///
/// # Parameters
/// - `publish`: JavaScript callback receiving each encoded message batch.
pub fn advance_graph_worker_run(publish: &Function) -> Result<bool, JsValue> {
    install_worker_panic_hook();
    let mut failure = None;
    let active = GRAPH_WORKER_RUNTIME.with(|runtime| {
        let mut runtime = runtime.borrow_mut();
        let runtime = runtime.as_mut().ok_or_else(|| {
            JsValue::from_str("graph-worker runtime was not initialized by the application")
        })?;
        Ok::<bool, JsValue>(runtime.advance_streaming(&mut |message| {
            publish_graph_message(publish, message, &mut failure);
        }))
    })?;
    failure.map_or(Ok(active), Err)
}

#[wasm_bindgen(js_name = takeBrowserOutputFiles)]
/// Drains completed browser-writer files for transfer to the page that owns downloads.
pub fn take_browser_output_files() -> Result<Vec<u8>, JsValue> {
    serde_json::to_vec(&super::web_output_storage::take_completed_files())
        .map_err(|error| JsValue::from_str(&format!("could not encode browser outputs: {error}")))
}

fn publish_capture_message(
    publish: &Function,
    message: signal_capture::CaptureWorkerMessage,
    failure: &mut Option<JsValue>,
) {
    if failure.is_some() {
        return;
    }
    let encoded = match encode_capture_worker_messages(&[message]) {
        Ok(encoded) => encoded,
        Err(error) => {
            *failure = Some(JsValue::from_str(&error));
            return;
        }
    };
    let bytes = Uint8Array::from(encoded.as_slice());
    if let Err(error) = publish.call1(&JsValue::UNDEFINED, &bytes) {
        *failure = Some(error);
    }
}

fn publish_graph_message(
    publish: &Function,
    message: logic_analyzer_graph_orchestration::GraphWorkerMessage,
    failure: &mut Option<JsValue>,
) {
    if failure.is_some() {
        return;
    }
    let encoded = match encode_graph_worker_messages(&[message]) {
        Ok(encoded) => encoded,
        Err(error) => {
            *failure = Some(JsValue::from_str(&error));
            return;
        }
    };
    let bytes = Uint8Array::from(encoded.as_slice());
    if let Err(error) = publish.call1(&JsValue::UNDEFINED, &bytes) {
        *failure = Some(error);
    }
}

#[wasm_bindgen(js_name = beginCaptureIdentity)]
/// Allocates an incremental BLAKE3 capture-identity handle.
///
/// Feed the handle with [`update_capture_identity`] and consume it exactly once with
/// [`finish_capture_identity`] or discard it with [`cancel_capture_identity`].
pub fn begin_capture_identity() -> Result<u32, JsValue> {
    CAPTURE_IDENTITY_HASHERS.with(|state| {
        let mut state = state.borrow_mut();
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| JsValue::from_str("capture identity handles are exhausted"))?;
        let id = state.next_id;
        state.hashers.insert(id, blake3::Hasher::new());
        Ok(id)
    })
}

#[wasm_bindgen(js_name = updateCaptureIdentity)]
/// Adds bytes to an incremental capture-identity hash.
///
/// # Parameters
/// - `id`: Handle returned by [`begin_capture_identity`].
/// - `bytes`: Next contiguous file-data bytes to hash.
pub fn update_capture_identity(id: u32, bytes: Vec<u8>) -> Result<(), JsValue> {
    CAPTURE_IDENTITY_HASHERS.with(|state| {
        let mut state = state.borrow_mut();
        let hasher = state
            .hashers
            .get_mut(&id)
            .ok_or_else(|| JsValue::from_str("capture identity handle does not exist"))?;
        hasher.update(&bytes);
        Ok(())
    })
}

#[wasm_bindgen(js_name = finishCaptureIdentity)]
/// Finalizes and removes an incremental capture-identity hash.
///
/// # Parameters
/// - `id`: Handle returned by [`begin_capture_identity`].
///
/// Returns the 32-byte BLAKE3 digest or an error when the handle is unknown.
pub fn finish_capture_identity(id: u32) -> Result<Vec<u8>, JsValue> {
    CAPTURE_IDENTITY_HASHERS.with(|state| {
        state
            .borrow_mut()
            .hashers
            .remove(&id)
            .map(|hasher| hasher.finalize().as_bytes().to_vec())
            .ok_or_else(|| JsValue::from_str("capture identity handle does not exist"))
    })
}

#[wasm_bindgen(js_name = cancelCaptureIdentity)]
/// Discards an incremental capture-identity hash without producing a digest.
///
/// # Parameters
/// - `id`: Handle returned by [`begin_capture_identity`]. Unknown handles are ignored.
pub fn cancel_capture_identity(id: u32) {
    CAPTURE_IDENTITY_HASHERS.with(|state| {
        state.borrow_mut().hashers.remove(&id);
    });
}

#[wasm_bindgen(js_name = inspectCaptureFile)]
/// Creates serialized capture metadata for an already-attached browser file.
///
/// # Parameters
/// - `reference`: Opaque file reference used by the browser import pipeline.
/// - `display_name`: User-facing file name.
/// - `identity`: 32-byte BLAKE3 content identity returned by the incremental hash API.
/// - `length`: Non-negative integral file length in bytes.
///
/// Returns JSON-encoded [`CaptureMetadata`] suitable for the worker attach response.
pub fn inspect_capture_file(
    reference: String,
    display_name: String,
    identity: Vec<u8>,
    length: f64,
) -> Result<Vec<u8>, JsValue> {
    install_worker_panic_hook();
    let identity: [u8; 32] = identity
        .try_into()
        .map_err(|_| JsValue::from_str("capture content identity must contain 32 bytes"))?;
    if !length.is_finite() || length < 0.0 || length.fract() != 0.0 {
        return Err(JsValue::from_str("browser capture length is invalid"));
    }
    let metadata = super::web_file_import::capture_metadata(
        reference,
        display_name,
        SourceIdentity::from_bytes(identity),
        length as u64,
    )
    .map_err(|error| JsValue::from_str(&error))?;
    serde_json::to_vec(&metadata)
        .map_err(|error| JsValue::from_str(&format!("could not encode capture metadata: {error}")))
}

fn install_worker_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let message = info.to_string();
            let _ = Reflect::set(
                &js_sys::global(),
                &JsValue::from_str(WORKER_PANIC_PROPERTY),
                &JsValue::from_str(&message),
            );
            console_error_panic_hook::hook(info);
        }));
    });
}

fn disconnect(
    capture_client: &Arc<CaptureWorkerClient>,
    graph_client: &Arc<GraphWorkerClient>,
    disconnected: &Rc<Cell<bool>>,
    message: String,
) {
    if !disconnected.replace(true) {
        capture_client.fail_all(message.clone());
        graph_client.fail_all(message);
    }
}

fn post_capture_request(worker: &Worker, request: &CaptureWorkerRequest) -> Result<(), String> {
    let payload = serde_json::to_vec(request)
        .map_err(|error| format!("could not encode capture-worker request: {error}"))?;
    let message = message_object("capture_run")?;
    let bytes = Uint8Array::from(payload.as_slice());
    let buffer = bytes.buffer();
    set(&message, "payload", buffer.clone().into())?;
    let transfer = Array::new();
    transfer.push(&buffer);
    worker
        .post_message_with_transfer(&message, &transfer)
        .map_err(|error| js_error("could not submit capture-worker request", error))
}

fn post_graph_request(worker: &Worker, request: &GraphWorkerRequest) -> Result<(), String> {
    let payload = encode_graph_worker_request(request)
        .map_err(|error| format!("could not encode graph-worker request: {error}"))?;
    let message = message_object("graph_run")?;
    let bytes = Uint8Array::from(payload.as_slice());
    let buffer = bytes.buffer();
    set(&message, "payload", buffer.clone().into())?;
    let transfer = Array::new();
    transfer.push(&buffer);
    worker
        .post_message_with_transfer(&message, &transfer)
        .map_err(|error| js_error("could not submit graph-worker request", error))
}

fn create_worker_url() -> Result<String, String> {
    let parts = Array::new();
    parts.push(&JsValue::from_str(WORKER_BOOTSTRAP));
    let options = BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &options)
        .map_err(|error| js_error("could not create capture-worker bootstrap", error))?;
    Url::create_object_url_with_blob(&blob)
        .map_err(|error| js_error("could not create capture-worker bootstrap URL", error))
}

fn create_worker(worker_url: &str) -> Result<Worker, String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    Worker::new_with_options(worker_url, &options)
        .map_err(|error| js_error("could not create capture worker", error))
}

fn post_initialize(worker: &Worker, module_url: &str, wasm_url: &str) -> Result<(), String> {
    let message = message_object("initialize")?;
    set(&message, "moduleUrl", JsValue::from_str(module_url))?;
    set(&message, "wasmUrl", JsValue::from_str(wasm_url))?;
    worker
        .post_message(&message)
        .map_err(|error| js_error("could not initialize capture worker", error))
}

fn message_object(kind: &str) -> Result<JsValue, String> {
    let message: JsValue = Object::new().into();
    set(&message, "kind", JsValue::from_str(kind))?;
    Ok(message)
}

fn property(object: &JsValue, name: &str) -> Result<JsValue, String> {
    Reflect::get(object, &JsValue::from_str(name))
        .map_err(|error| js_error(&format!("worker message has no '{name}' property"), error))
}

fn set(object: &JsValue, name: &str, value: JsValue) -> Result<(), String> {
    Reflect::set(object, &JsValue::from_str(name), &value)
        .map(|_| ())
        .map_err(|error| js_error(&format!("could not set worker message '{name}'"), error))
}

fn string_property(object: &JsValue, name: &str) -> Result<String, String> {
    property(object, name)?
        .as_string()
        .ok_or_else(|| format!("worker message '{name}' is not a string"))
}

fn integer_property(object: &JsValue, name: &str) -> Result<u64, String> {
    string_property(object, name)?
        .parse()
        .map_err(|_| format!("worker message '{name}' is not an unsigned integer"))
}

fn js_error(context: &str, error: JsValue) -> String {
    let detail = error.as_string().unwrap_or_else(|| format!("{error:?}"));
    format!("{context}: {detail}")
}

#[cfg(test)]
mod web_capture_worker_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{begin_capture_identity, finish_capture_identity, update_capture_identity};

    #[wasm_bindgen_test(unsupported = test)]
    fn incremental_worker_identity_matches_the_shared_blake3_identity() {
        let id = begin_capture_identity().unwrap();
        update_capture_identity(id, b"capture ".to_vec()).unwrap();
        update_capture_identity(id, b"bytes".to_vec()).unwrap();

        assert_eq!(
            finish_capture_identity(id).unwrap(),
            blake3::hash(b"capture bytes").as_bytes()
        );
    }
}
