use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker, WorkerOptions, WorkerType,
};

use signal_processing::portable_worker_kernels;
use signal_runtime::{
    WorkerExecutionCapability, WorkerHostCommand, WorkerMessage, WorkerOperation,
    WorkerOperationExecutor, WorkerOperationQueue, WorkerRequest,
};

const WORKER_BOOTSTRAP: &str = include_str!("web_worker_bootstrap.js");

thread_local! {
    static PORTABLE_KERNELS: signal_runtime::WorkerKernelRegistry = portable_worker_kernels();
}

struct AdapterState {
    workers: Vec<Worker>,
    queue: WorkerOperationQueue,
    module_url: String,
    wasm_url: String,
    initialization_started: bool,
}

/// Browser Web Worker pool for portable finite processing operations.
///
/// Construction creates the workers and starts loading the generated WASM
/// module. Requests submitted before the workers report readiness remain in a
/// bounded queue. Terminal messages are exposed in submission order even when
/// workers complete out of order.
pub struct WebWorkerAdapter {
    state: Rc<RefCell<AdapterState>>,
    message_handlers: Vec<Closure<dyn FnMut(MessageEvent)>>,
    error_handlers: Vec<Closure<dyn FnMut(ErrorEvent)>>,
}

impl WebWorkerAdapter {
    /// Creates a worker pool for the generated JS module and WASM binary.
    ///
    /// # Parameters
    /// - `module_url`: Input consumed by this operation.
    /// - `wasm_url`: Input consumed by this operation.
    /// - `worker_count`: Input consumed by this operation.
    /// - `max_outstanding`: Input consumed by this operation.
    /// - `required_operations`: Input consumed by this operation.
    pub fn new(
        module_url: &str,
        wasm_url: &str,
        worker_count: usize,
        max_outstanding: usize,
        required_operations: &[WorkerOperation],
    ) -> Result<Self, String> {
        if worker_count == 0 {
            return Err("the Web Worker pool must contain at least one worker".to_string());
        }
        if max_outstanding < worker_count {
            return Err(
                "the Web Worker queue must hold at least one request per worker".to_string(),
            );
        }
        let kernels = portable_worker_kernels();
        let operations = kernels.operations().cloned().collect::<Vec<_>>();
        if let Some(operation) = required_operations
            .iter()
            .find(|operation| !kernels.supports(operation))
        {
            return Err(format!(
                "Web Worker operation '{}' is not registered",
                operation.as_str()
            ));
        }

        let worker_url = create_worker_url()?;
        let mut workers: Vec<Worker> = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let worker = match create_worker(&worker_url) {
                Ok(worker) => worker,
                Err(error) => {
                    for worker in &workers {
                        worker.terminate();
                    }
                    let _ = Url::revoke_object_url(&worker_url);
                    return Err(error);
                }
            };
            workers.push(worker);
        }
        if let Err(error) = Url::revoke_object_url(&worker_url) {
            for worker in &workers {
                worker.terminate();
            }
            return Err(js_error("could not release worker bootstrap URL", error));
        }

        let queue = WorkerOperationQueue::new(worker_count, max_outstanding, operations)?;
        let state = Rc::new(RefCell::new(AdapterState {
            workers,
            queue,
            module_url: module_url.to_string(),
            wasm_url: wasm_url.to_string(),
            initialization_started: false,
        }));
        let mut message_handlers = Vec::with_capacity(worker_count);
        let mut error_handlers = Vec::with_capacity(worker_count);

        for worker_index in 0..worker_count {
            let message_state = Rc::clone(&state);
            let message_handler =
                Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                    handle_worker_message(&message_state, worker_index, event.data());
                });
            let error_state = Rc::clone(&state);
            let error_handler = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
                event.prevent_default();
                handle_worker_error(&error_state, worker_index, event.message());
            });
            let worker = &state.borrow().workers[worker_index];
            worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));
            message_handlers.push(message_handler);
            error_handlers.push(error_handler);
        }

        Ok(Self {
            state,
            message_handlers,
            error_handlers,
        })
    }

    /// Number of browser workers owned by the adapter.
    pub fn available_parallelism(&self) -> usize {
        self.state.borrow().queue.available_parallelism()
    }

    /// Adds a finite request to the bounded worker queue.
    ///
    /// # Parameters
    /// - `request`: Input consumed by this operation.
    pub fn submit(&self, request: WorkerRequest) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        let commands = state.queue.submit(request)?;
        initialize_workers(&mut state);
        apply_commands(&mut state, commands);
        Ok(())
    }

    /// Cancels a queued or running request at the host boundary.
    ///
    /// A synchronous kernel may finish its current operation, but its result
    /// is discarded and cancellation is released in submission order.
    pub fn cancel(&self, sequence: u64) -> bool {
        let mut state = self.state.borrow_mut();
        let (accepted, commands) = state.queue.cancel(sequence);
        apply_commands(&mut state, commands);
        accepted
    }

    /// Drains progress and deterministically ordered terminal messages.
    pub fn drain_messages(&self) -> Vec<WorkerMessage> {
        self.state.borrow_mut().queue.drain_messages()
    }

    /// Number of queued or running requests awaiting ordered delivery.
    pub fn outstanding(&self) -> usize {
        self.state.borrow().queue.outstanding()
    }
}

impl WorkerOperationExecutor for WebWorkerAdapter {
    fn capability(&self) -> WorkerExecutionCapability {
        self.state.borrow().queue.capability()
    }

    fn submit(&self, request: WorkerRequest) -> Result<(), String> {
        WebWorkerAdapter::submit(self, request)
    }

    fn cancel(&self, sequence: u64) -> bool {
        WebWorkerAdapter::cancel(self, sequence)
    }

    fn drain_messages(&self) -> Vec<WorkerMessage> {
        WebWorkerAdapter::drain_messages(self)
    }

    fn outstanding(&self) -> usize {
        WebWorkerAdapter::outstanding(self)
    }
}

impl Drop for WebWorkerAdapter {
    fn drop(&mut self) {
        for worker in &self.state.borrow().workers {
            worker.terminate();
        }
        self.message_handlers.clear();
        self.error_handlers.clear();
    }
}

#[wasm_bindgen(js_name = executePortableWorkerOperation)]
/// Executes one portable worker operation received from the browser bootstrap.
pub fn execute_portable_worker_operation(
    operation: String,
    payload: Vec<u8>,
) -> Result<Vec<u8>, JsValue> {
    let operation =
        WorkerOperation::new(operation).map_err(|error| JsValue::from_str(&error.to_string()))?;
    let message = PORTABLE_KERNELS.with(|kernels| {
        kernels.execute(WorkerRequest {
            sequence: 0,
            operation,
            payload,
        })
    });
    match message {
        WorkerMessage::Complete { payload, .. } => Ok(payload),
        WorkerMessage::Failed { message, .. } => Err(JsValue::from_str(&message)),
        _ => Err(JsValue::from_str(
            "worker kernel returned a non-terminal message",
        )),
    }
}

fn create_worker_url() -> Result<String, String> {
    let parts = Array::new();
    parts.push(&JsValue::from_str(WORKER_BOOTSTRAP));
    let options = BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &options)
        .map_err(|error| js_error("could not create worker bootstrap", error))?;
    Url::create_object_url_with_blob(&blob)
        .map_err(|error| js_error("could not create worker bootstrap URL", error))
}

fn create_worker(worker_url: &str) -> Result<Worker, String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    Worker::new_with_options(worker_url, &options)
        .map_err(|error| js_error("could not create Web Worker", error))
}

fn post_initialize(worker: &Worker, module_url: &str, wasm_url: &str) -> Result<(), String> {
    let message = message_object("initialize")?;
    set(&message, "moduleUrl", JsValue::from_str(module_url))?;
    set(&message, "wasmUrl", JsValue::from_str(wasm_url))?;
    worker
        .post_message(&message)
        .map_err(|error| js_error("could not initialize Web Worker", error))
}

fn post_run(worker: &Worker, request: WorkerRequest) -> Result<(), String> {
    let message = message_object("run")?;
    set(
        &message,
        "sequence",
        JsValue::from_str(&request.sequence.to_string()),
    )?;
    set(
        &message,
        "operation",
        JsValue::from_str(request.operation.as_str()),
    )?;
    let bytes = Uint8Array::from(request.payload.as_slice());
    let buffer = bytes.buffer();
    set(&message, "payload", buffer.clone().into())?;
    let transfer = Array::new();
    transfer.push(&buffer);
    worker
        .post_message_with_transfer(&message, &transfer)
        .map_err(|error| js_error("could not submit Web Worker operation", error))
}

fn post_cancel(worker: &Worker, sequence: u64) -> Result<(), String> {
    let message = message_object("cancel")?;
    set(
        &message,
        "sequence",
        JsValue::from_str(&sequence.to_string()),
    )?;
    worker
        .post_message(&message)
        .map_err(|error| js_error("could not cancel Web Worker operation", error))
}

fn initialize_workers(state: &mut AdapterState) {
    if state.initialization_started {
        return;
    }
    state.initialization_started = true;
    let mut commands = Vec::new();
    for worker_index in 0..state.workers.len() {
        if let Err(error) = post_initialize(
            &state.workers[worker_index],
            &state.module_url,
            &state.wasm_url,
        ) {
            commands.extend(state.queue.worker_failed(worker_index, error));
        }
    }
    apply_commands(state, commands);
}

fn apply_commands(state: &mut AdapterState, commands: Vec<WorkerHostCommand>) {
    let mut commands = commands.into_iter().collect::<VecDeque<_>>();
    while let Some(command) = commands.pop_front() {
        match command {
            WorkerHostCommand::Run {
                worker_index,
                request,
            } => {
                let result = state
                    .workers
                    .get(worker_index)
                    .ok_or_else(|| format!("worker slot {worker_index} does not exist"))
                    .and_then(|worker| post_run(worker, request));
                if let Err(message) = result {
                    commands.extend(state.queue.worker_failed(worker_index, message));
                }
            }
            WorkerHostCommand::Cancel {
                worker_index,
                sequence,
            } => {
                if let Some(worker) = state.workers.get(worker_index) {
                    let _ = post_cancel(worker, sequence);
                }
            }
        }
    }
}

fn handle_worker_message(state: &Rc<RefCell<AdapterState>>, worker_index: usize, value: JsValue) {
    let kind = string_property(&value, "kind").unwrap_or_default();
    let mut state = state.borrow_mut();
    let commands = match kind.as_str() {
        "ready" => state.queue.worker_ready(worker_index),
        "progress" => {
            if let (Ok(sequence), Ok(completed)) = (
                sequence_property(&value),
                integer_property(&value, "completed"),
            ) {
                let total = optional_integer_property(&value, "total").ok().flatten();
                state.queue.worker_progress(sequence, completed, total);
            }
            Vec::new()
        }
        "complete" | "failed" => {
            let sequence = sequence_property(&value).ok();
            let result = if kind == "complete" {
                property(&value, "payload").map(|payload| Uint8Array::new(&payload).to_vec())
            } else {
                Err(string_property(&value, "message").unwrap_or_else(|error| error))
            };
            state.queue.worker_completed(worker_index, sequence, result)
        }
        "worker_failed" => {
            let message = string_property(&value, "message").unwrap_or_else(|error| error);
            state.queue.worker_failed(worker_index, message)
        }
        _ => state.queue.worker_failed(
            worker_index,
            "Web Worker returned an unknown message".to_string(),
        ),
    };
    apply_commands(&mut state, commands);
}

fn handle_worker_error(state: &Rc<RefCell<AdapterState>>, worker_index: usize, message: String) {
    let mut state = state.borrow_mut();
    let commands = state.queue.worker_failed(worker_index, message);
    apply_commands(&mut state, commands);
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

fn sequence_property(object: &JsValue) -> Result<u64, String> {
    string_property(object, "sequence")?
        .parse()
        .map_err(|_| "worker message sequence is invalid".to_string())
}

fn integer_property(object: &JsValue, name: &str) -> Result<u64, String> {
    let value = property(object, name)?;
    if let Some(value) = value.as_string() {
        return value
            .parse()
            .map_err(|_| format!("worker message '{name}' is not an unsigned integer"));
    }
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
        .ok_or_else(|| format!("worker message '{name}' is not an unsigned integer"))
}

fn optional_integer_property(object: &JsValue, name: &str) -> Result<Option<u64>, String> {
    let value = property(object, name)?;
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        integer_property(object, name).map(Some)
    }
}

fn js_error(context: &str, error: JsValue) -> String {
    let detail = error.as_string().unwrap_or_else(|| format!("{error:?}"));
    format!("{context}: {detail}")
}
