use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker, WorkerOptions, WorkerType,
};

use signal_processing::{WorkerMessage, WorkerOperation, WorkerRequest, portable_worker_kernels};

const WORKER_BOOTSTRAP: &str = include_str!("web_worker_bootstrap.js");

thread_local! {
    static PORTABLE_KERNELS: signal_processing::WorkerKernelRegistry = portable_worker_kernels();
}

struct WorkerSlot {
    worker: Worker,
    ready: bool,
    failed: bool,
    running: Option<u64>,
}

struct AdapterState {
    workers: Vec<WorkerSlot>,
    pending: VecDeque<WorkerRequest>,
    submission_order: VecDeque<u64>,
    terminal: BTreeMap<u64, WorkerMessage>,
    delivered: VecDeque<WorkerMessage>,
    cancelled: BTreeSet<u64>,
    max_outstanding: usize,
    last_submitted_sequence: Option<u64>,
}

impl AdapterState {
    fn outstanding(&self) -> usize {
        self.submission_order.len()
    }

    fn contains_sequence(&self, sequence: u64) -> bool {
        self.submission_order.contains(&sequence)
    }

    fn record_terminal(&mut self, message: WorkerMessage) {
        let sequence = terminal_sequence(&message);
        if self.contains_sequence(sequence) && !self.terminal.contains_key(&sequence) {
            self.terminal.insert(sequence, message);
        }
        self.release_ordered();
    }

    fn release_ordered(&mut self) {
        while let Some(sequence) = self.submission_order.front().copied() {
            let Some(message) = self.terminal.remove(&sequence) else {
                break;
            };
            self.submission_order.pop_front();
            self.cancelled.remove(&sequence);
            self.delivered.push_back(message);
        }
    }
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
    pub fn new(
        module_url: &str,
        wasm_url: &str,
        worker_count: usize,
        max_outstanding: usize,
    ) -> Result<Self, String> {
        if worker_count == 0 {
            return Err("the Web Worker pool must contain at least one worker".to_string());
        }
        if max_outstanding < worker_count {
            return Err(
                "the Web Worker queue must hold at least one request per worker".to_string(),
            );
        }

        let worker_url = create_worker_url()?;
        let mut workers: Vec<WorkerSlot> = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let worker = match create_worker(&worker_url) {
                Ok(worker) => worker,
                Err(error) => {
                    for slot in &workers {
                        slot.worker.terminate();
                    }
                    let _ = Url::revoke_object_url(&worker_url);
                    return Err(error);
                }
            };
            workers.push(WorkerSlot {
                worker,
                ready: false,
                failed: false,
                running: None,
            });
        }
        if let Err(error) = Url::revoke_object_url(&worker_url) {
            for slot in &workers {
                slot.worker.terminate();
            }
            return Err(js_error("could not release worker bootstrap URL", error));
        }

        let state = Rc::new(RefCell::new(AdapterState {
            workers,
            pending: VecDeque::new(),
            submission_order: VecDeque::new(),
            terminal: BTreeMap::new(),
            delivered: VecDeque::new(),
            cancelled: BTreeSet::new(),
            max_outstanding,
            last_submitted_sequence: None,
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
            let worker = &state.borrow().workers[worker_index].worker;
            worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));
            message_handlers.push(message_handler);
            error_handlers.push(error_handler);
        }

        let adapter = Self {
            state,
            message_handlers,
            error_handlers,
        };
        let workers = adapter
            .state
            .borrow()
            .workers
            .iter()
            .map(|slot| slot.worker.clone())
            .collect::<Vec<_>>();
        for worker in workers {
            post_initialize(&worker, module_url, wasm_url)?;
        }
        Ok(adapter)
    }

    /// Number of browser workers owned by the adapter.
    pub fn available_parallelism(&self) -> usize {
        self.state.borrow().workers.len()
    }

    /// Adds a finite request to the bounded worker queue.
    pub fn submit(&self, request: WorkerRequest) -> Result<(), String> {
        let mut state = self.state.borrow_mut();
        if state
            .last_submitted_sequence
            .is_some_and(|previous| request.sequence <= previous)
        {
            return Err(format!(
                "worker request sequence {} is not greater than the previous sequence",
                request.sequence
            ));
        }
        if state.outstanding() >= state.max_outstanding {
            return Err("Web Worker request queue is full".to_string());
        }
        state.last_submitted_sequence = Some(request.sequence);
        state.submission_order.push_back(request.sequence);
        state.pending.push_back(request);
        dispatch_ready_workers(&mut state);
        Ok(())
    }

    /// Cancels a queued or running request at the host boundary.
    ///
    /// A synchronous kernel may finish its current operation, but its result
    /// is discarded and cancellation is released in submission order.
    pub fn cancel(&self, sequence: u64) -> bool {
        let mut state = self.state.borrow_mut();
        if !state.contains_sequence(sequence) {
            return false;
        }
        state.cancelled.insert(sequence);
        if let Some(index) = state
            .pending
            .iter()
            .position(|request| request.sequence == sequence)
        {
            state.pending.remove(index);
        }
        for slot in &state.workers {
            if slot.running == Some(sequence) {
                let _ = post_cancel(&slot.worker, sequence);
            }
        }
        state.record_terminal(WorkerMessage::Failed {
            sequence,
            message: "worker operation was cancelled".to_string(),
        });
        dispatch_ready_workers(&mut state);
        true
    }

    /// Drains progress and deterministically ordered terminal messages.
    pub fn drain_messages(&self) -> Vec<WorkerMessage> {
        self.state.borrow_mut().delivered.drain(..).collect()
    }

    /// Number of queued or running requests awaiting ordered delivery.
    pub fn outstanding(&self) -> usize {
        self.state.borrow().outstanding()
    }
}

impl Drop for WebWorkerAdapter {
    fn drop(&mut self) {
        for slot in &self.state.borrow().workers {
            slot.worker.terminate();
        }
        self.message_handlers.clear();
        self.error_handlers.clear();
    }
}

#[wasm_bindgen(js_name = executePortableWorkerOperation)]
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

fn dispatch_ready_workers(state: &mut AdapterState) {
    for slot in &mut state.workers {
        if !slot.ready || slot.failed || slot.running.is_some() {
            continue;
        }
        let Some(request) = state.pending.pop_front() else {
            break;
        };
        let sequence = request.sequence;
        match post_run(&slot.worker, request) {
            Ok(()) => slot.running = Some(sequence),
            Err(message) => {
                slot.ready = false;
                slot.failed = true;
                state
                    .terminal
                    .insert(sequence, WorkerMessage::Failed { sequence, message });
            }
        }
    }
    fail_pending_if_unavailable(state);
    state.release_ordered();
}

fn handle_worker_message(state: &Rc<RefCell<AdapterState>>, worker_index: usize, value: JsValue) {
    let kind = string_property(&value, "kind").unwrap_or_default();
    let mut state = state.borrow_mut();
    match kind.as_str() {
        "ready" => {
            if !state.workers[worker_index].failed {
                state.workers[worker_index].ready = true;
            }
        }
        "progress" => {
            if let (Ok(sequence), Ok(completed)) = (
                sequence_property(&value),
                integer_property(&value, "completed"),
            ) {
                let total = optional_integer_property(&value, "total").ok().flatten();
                if state.contains_sequence(sequence) && !state.cancelled.contains(&sequence) {
                    state.delivered.push_back(WorkerMessage::Progress {
                        sequence,
                        completed,
                        total,
                    });
                }
            }
        }
        "complete" | "failed" => {
            let running = state.workers[worker_index].running.take();
            let sequence = sequence_property(&value).ok().or(running);
            if let Some(sequence) = sequence
                && !state.cancelled.contains(&sequence)
            {
                let message = if kind == "complete" {
                    match property(&value, "payload") {
                        Ok(payload) => WorkerMessage::Complete {
                            sequence,
                            payload: Uint8Array::new(&payload).to_vec(),
                        },
                        Err(message) => WorkerMessage::Failed { sequence, message },
                    }
                } else {
                    WorkerMessage::Failed {
                        sequence,
                        message: string_property(&value, "message").unwrap_or_else(|error| error),
                    }
                };
                state.record_terminal(message);
            }
        }
        "worker_failed" => {
            let message = string_property(&value, "message").unwrap_or_else(|error| error);
            fail_worker(&mut state, worker_index, message);
        }
        _ => fail_worker(
            &mut state,
            worker_index,
            "Web Worker returned an unknown message".to_string(),
        ),
    }
    dispatch_ready_workers(&mut state);
}

fn handle_worker_error(state: &Rc<RefCell<AdapterState>>, worker_index: usize, message: String) {
    let mut state = state.borrow_mut();
    fail_worker(&mut state, worker_index, message);
    dispatch_ready_workers(&mut state);
}

fn fail_worker(state: &mut AdapterState, worker_index: usize, message: String) {
    let slot = &mut state.workers[worker_index];
    slot.ready = false;
    slot.failed = true;
    if let Some(sequence) = slot.running.take()
        && !state.cancelled.contains(&sequence)
    {
        state.record_terminal(WorkerMessage::Failed { sequence, message });
    }
    fail_pending_if_unavailable(state);
}

fn fail_pending_if_unavailable(state: &mut AdapterState) {
    if state.workers.iter().all(|slot| slot.failed) {
        while let Some(request) = state.pending.pop_front() {
            let sequence = request.sequence;
            state.terminal.insert(
                sequence,
                WorkerMessage::Failed {
                    sequence,
                    message: "all Web Workers are unavailable".to_string(),
                },
            );
        }
        state.release_ordered();
    }
}

fn terminal_sequence(message: &WorkerMessage) -> u64 {
    match message {
        WorkerMessage::Complete { sequence, .. } | WorkerMessage::Failed { sequence, .. } => {
            *sequence
        }
        _ => unreachable!("only terminal messages enter the ordered completion buffer"),
    }
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
