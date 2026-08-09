use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, ErrorEvent, MessageEvent, Url, Worker, WorkerOptions, WorkerType,
};

use platform_artifacts::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, MemoryArtifactRepository,
    ReadArtifact, RepositoryCapabilities, RepositoryError, SourceIdentity, WriteArtifact,
};

use crate::{ArtifactRepositoryOpenError, ArtifactRepositoryOpenOperation};

const OPFS_WORKER_BOOTSTRAP: &str = include_str!("opfs_worker_bootstrap.js");
const DEFAULT_BROWSER_CACHE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_QUEUED_PERSISTENCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_IN_FLIGHT_COMMANDS: usize = 4;
const PUMP_INTERVAL_MS: i32 = 4;

#[derive(Debug, thiserror::Error)]
enum BrowserArtifactProtocolError {
    #[error("persistence-worker message has no '{property}' property: {detail}")]
    PropertyAccess { property: String, detail: String },
    #[error("could not set persistence-worker message '{property}': {detail}")]
    PropertyWrite { property: String, detail: String },
    #[error("persistence-worker message '{property}' is not {expected}")]
    InvalidProperty {
        property: String,
        expected: &'static str,
    },
    #[error("persistence-worker entry has an invalid namespace: {0}")]
    Namespace(#[source] RepositoryError),
    #[error("browser artifact identity has an invalid length")]
    IdentityLength,
    #[error("browser artifact identity contains invalid hexadecimal")]
    IdentityHex,
    #[error("persistence worker returned unexpected '{kind}' message")]
    UnexpectedMessage { kind: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrowserPersistenceOperation {
    Publish,
    Remove,
}

impl fmt::Display for BrowserPersistenceOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Publish => "publish browser artifact",
            Self::Remove => "remove browser artifact",
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum BrowserPersistenceCommandError {
    #[error("invalid persistence-worker command: {0}")]
    Protocol(#[from] BrowserArtifactProtocolError),
    #[error("could not {operation}: {detail}")]
    Submission {
        operation: BrowserPersistenceOperation,
        detail: String,
    },
}

thread_local! {
    static OPFS_RUNTIMES: RefCell<Vec<OpfsRuntime>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
pub(crate) struct BrowserArtifactRepository {
    memory: MemoryArtifactRepository,
    mirror: Arc<Mutex<MirrorState>>,
}

struct InitialState {
    durable: bool,
    quota: Option<u64>,
    usage: Option<u64>,
    evicted: u64,
    entries: Vec<InitialEntry>,
}

struct InitialEntry {
    key: ArtifactKey,
    bytes: Vec<u8>,
}

struct MirrorState {
    commands: VecDeque<PersistenceCommand>,
    queued_bytes: u64,
    in_flight: usize,
    next_sequence: u64,
    durable: bool,
    quota: Option<u64>,
    usage: Option<u64>,
}

enum PersistenceCommand {
    Publish {
        sequence: u64,
        key: ArtifactKey,
        bytes: Vec<u8>,
    },
    Remove {
        sequence: u64,
        key: ArtifactKey,
    },
}

struct OpfsRuntime {
    _worker: Worker,
    _message_handler: Closure<dyn FnMut(MessageEvent)>,
    _error_handler: Closure<dyn FnMut(ErrorEvent)>,
    _pump: Closure<dyn FnMut()>,
    _interval_id: i32,
}

impl BrowserArtifactRepository {
    pub(crate) async fn open(root_name: &str) -> Result<Self, ArtifactRepositoryOpenError> {
        Self::open_with_budget(root_name, DEFAULT_BROWSER_CACHE_BYTES).await
    }

    async fn open_with_budget(
        root_name: &str,
        max_bytes: u64,
    ) -> Result<Self, ArtifactRepositoryOpenError> {
        let (worker, initial) = initialize_worker(root_name, max_bytes).await?;
        let memory = MemoryArtifactRepository::with_budget(max_bytes);
        if let Err(error) = hydrate_entries(&memory, initial.entries) {
            worker.terminate();
            return Err(error);
        }
        if initial.evicted > 0 {
            tracing::warn!(
                evicted = initial.evicted,
                max_bytes,
                "browser persistence evicted cache entries while hydrating the session"
            );
        }
        tracing::debug!(
            durable = initial.durable,
            quota_bytes = initial.quota,
            usage_bytes = initial.usage,
            "browser artifact repository hydrated from OPFS"
        );
        let mirror = Arc::new(Mutex::new(MirrorState {
            commands: VecDeque::new(),
            queued_bytes: 0,
            in_flight: 0,
            next_sequence: 1,
            durable: initial.durable,
            quota: initial.quota,
            usage: initial.usage,
        }));
        install_runtime(worker, Arc::clone(&mirror))?;
        Ok(Self { memory, mirror })
    }

    fn enqueue_publish(&self, key: ArtifactKey, bytes: Vec<u8>) -> Result<(), RepositoryError> {
        let byte_count = u64::try_from(bytes.len()).map_err(|_| RepositoryError::QuotaExceeded)?;
        let mut mirror = self
            .mirror
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        if byte_count > MAX_QUEUED_PERSISTENCE_BYTES
            || mirror.queued_bytes > MAX_QUEUED_PERSISTENCE_BYTES.saturating_sub(byte_count)
        {
            tracing::warn!(
                artifact_bytes = byte_count,
                queued_bytes = mirror.queued_bytes,
                "browser persistence queue is full; keeping this cache generation in session memory"
            );
            return Ok(());
        }
        let sequence = mirror.next_sequence;
        mirror.next_sequence = mirror.next_sequence.saturating_add(1);
        mirror.queued_bytes += byte_count;
        mirror.commands.push_back(PersistenceCommand::Publish {
            sequence,
            key,
            bytes,
        });
        Ok(())
    }

    fn enqueue_remove(&self, key: ArtifactKey) -> Result<(), RepositoryError> {
        let mut mirror = self
            .mirror
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        let sequence = mirror.next_sequence;
        mirror.next_sequence = mirror.next_sequence.saturating_add(1);
        mirror
            .commands
            .push_back(PersistenceCommand::Remove { sequence, key });
        Ok(())
    }

    #[cfg(test)]
    fn idle(&self) -> bool {
        self.mirror
            .lock()
            .map(|mirror| mirror.commands.is_empty() && mirror.in_flight == 0)
            .unwrap_or(false)
    }
}

fn hydrate_entries(
    repository: &MemoryArtifactRepository,
    entries: Vec<InitialEntry>,
) -> Result<(), ArtifactRepositoryOpenError> {
    for entry in entries {
        let mut writer = repository.begin_write(entry.key).map_err(hydration_error)?;
        writer.write_at(0, &entry.bytes).map_err(hydration_error)?;
        writer.publish().map_err(hydration_error)?;
    }
    Ok(())
}

impl ArtifactRepository for BrowserArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        let durable = self
            .mirror
            .lock()
            .map(|mirror| mirror.durable)
            .unwrap_or(false);
        RepositoryCapabilities {
            durable,
            atomic_publication: true,
            immutable_regions: true,
        }
    }

    fn namespaces(&self) -> Result<Vec<ArtifactNamespace>, RepositoryError> {
        self.memory.namespaces()
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        self.memory.open(key)
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        let writer = self.memory.begin_write(key.clone())?;
        Ok(Box::new(BrowserWriteArtifact {
            repository: self.clone(),
            writer: Some(writer),
            key,
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        self.memory.remove(key)?;
        self.enqueue_remove(key.clone())
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        self.memory.entries(namespace)
    }
}

struct BrowserWriteArtifact {
    repository: BrowserArtifactRepository,
    writer: Option<Box<dyn WriteArtifact>>,
    key: ArtifactKey,
}

impl BrowserWriteArtifact {
    fn writer(&mut self) -> Result<&mut Box<dyn WriteArtifact>, RepositoryError> {
        self.writer.as_mut().ok_or(RepositoryError::Unavailable)
    }
}

impl WriteArtifact for BrowserWriteArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        self.writer()?.write_at(offset, source)
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        self.writer()?.truncate(len)
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        self.writer()?.flush()
    }

    fn publish(mut self: Box<Self>) -> Result<(), RepositoryError> {
        self.writer
            .take()
            .ok_or(RepositoryError::Unavailable)?
            .publish()?;
        let bytes = read_complete_artifact(&self.repository.memory, &self.key)?;
        self.repository.enqueue_publish(self.key.clone(), bytes)
    }
}

fn read_complete_artifact(
    repository: &MemoryArtifactRepository,
    key: &ArtifactKey,
) -> Result<Vec<u8>, RepositoryError> {
    let mut reader = repository.open(key)?.ok_or_else(|| {
        RepositoryError::Corrupt("published browser artifact is missing from memory".into())
    })?;
    let length = usize::try_from(reader.len()?).map_err(|_| RepositoryError::QuotaExceeded)?;
    let mut bytes = vec![0_u8; length];
    let mut copied = 0;
    while copied < bytes.len() {
        let count = reader.read_at(copied as u64, &mut bytes[copied..])?;
        if count == 0 {
            return Err(RepositoryError::Corrupt(
                "published browser artifact is truncated".into(),
            ));
        }
        copied += count;
    }
    Ok(bytes)
}

async fn initialize_worker(
    root_name: &str,
    max_bytes: u64,
) -> Result<(Worker, InitialState), ArtifactRepositoryOpenError> {
    if root_name.is_empty() {
        return Err(ArtifactRepositoryOpenError::InvalidRootName);
    }
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
        return Err(host_js_error(
            ArtifactRepositoryOpenOperation::ReleaseWorkerUrl,
            error,
        ));
    }

    let callbacks = Rc::new(RefCell::new(None::<(Function, Function)>));
    let promise_callbacks = Rc::clone(&callbacks);
    let promise = Promise::new(&mut move |resolve, reject| {
        *promise_callbacks.borrow_mut() = Some((resolve.clone(), reject.clone()));
    });
    let message_callbacks = Rc::clone(&callbacks);
    let message_handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some((resolve, _reject)) = message_callbacks.borrow_mut().take() {
            let _ = resolve.call1(&JsValue::UNDEFINED, &event.data());
        }
    });
    let error_callbacks = Rc::clone(&callbacks);
    let error_handler = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        event.prevent_default();
        if let Some((_resolve, reject)) = error_callbacks.borrow_mut().take() {
            let _ = reject.call1(&JsValue::UNDEFINED, &JsValue::from_str(&event.message()));
        }
    });
    worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));
    let message = (|| -> Result<JsValue, BrowserArtifactProtocolError> {
        let message = message_object("initialize")?;
        set(&message, "rootName", JsValue::from_str(root_name))?;
        set(&message, "maxBytes", JsValue::from_f64(max_bytes as f64))?;
        Ok(message)
    })();
    let message = match message {
        Ok(message) => message,
        Err(error) => {
            worker.terminate();
            return Err(host_error(
                ArtifactRepositoryOpenOperation::SendInitialization,
                error.to_string(),
            ));
        }
    };
    if let Err(error) = worker.post_message(&message) {
        worker.terminate();
        return Err(host_js_error(
            ArtifactRepositoryOpenOperation::SendInitialization,
            error,
        ));
    }
    let value = match JsFuture::from(promise).await {
        Ok(value) => value,
        Err(error) => {
            worker.terminate();
            return Err(host_js_error(
                ArtifactRepositoryOpenOperation::AwaitInitialization,
                error,
            ));
        }
    };
    worker.set_onmessage(None);
    worker.set_onerror(None);
    drop(message_handler);
    drop(error_handler);
    match string_property(&value, "kind") {
        Ok(kind) if kind == "ready" => match parse_initial_state(&value) {
            Ok(state) => Ok((worker, state)),
            Err(error) => {
                worker.terminate();
                Err(error)
            }
        },
        Ok(kind) if kind == "unavailable" => {
            worker.terminate();
            string_property(&value, "message")
                .map(|message| ArtifactRepositoryOpenError::Unavailable { message })
                .map_or_else(|error| Err(protocol_error(error)), Err)
        }
        Ok(kind) => {
            worker.terminate();
            Err(protocol_error(
                BrowserArtifactProtocolError::UnexpectedMessage { kind },
            ))
        }
        Err(error) => {
            worker.terminate();
            Err(protocol_error(error))
        }
    }
}

fn parse_initial_state(value: &JsValue) -> Result<InitialState, ArtifactRepositoryOpenError> {
    (|| -> Result<InitialState, BrowserArtifactProtocolError> {
        let entries = Array::from(&property(value, "entries")?)
            .iter()
            .map(|entry| {
                let namespace = ArtifactNamespace::new(string_property(&entry, "namespace")?)
                    .map_err(BrowserArtifactProtocolError::Namespace)?;
                let identity = decode_identity(&string_property(&entry, "identity")?)?;
                let bytes = Uint8Array::new(&property(&entry, "bytes")?).to_vec();
                Ok(InitialEntry {
                    key: ArtifactKey::new(namespace, SourceIdentity::from_bytes(identity)),
                    bytes,
                })
            })
            .collect::<Result<Vec<_>, BrowserArtifactProtocolError>>()?;
        Ok(InitialState {
            durable: bool_property(value, "durable")?,
            quota: optional_integer_property(value, "quota")?,
            usage: optional_integer_property(value, "usage")?,
            evicted: integer_property(value, "evicted")?,
            entries,
        })
    })()
    .map_err(protocol_error)
}

fn install_runtime(
    worker: Worker,
    mirror: Arc<Mutex<MirrorState>>,
) -> Result<(), ArtifactRepositoryOpenError> {
    let message_mirror = Arc::clone(&mirror);
    let message_handler = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        handle_worker_message(&message_mirror, event.data());
    });
    let error_mirror = Arc::clone(&mirror);
    let error_handler = Closure::<dyn FnMut(ErrorEvent)>::new(move |event: ErrorEvent| {
        event.prevent_default();
        if let Ok(mut mirror) = error_mirror.lock() {
            mirror.durable = false;
            mirror.in_flight = 0;
        }
        tracing::warn!(message = %event.message(), "browser persistence worker failed");
    });
    worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));

    let pump_worker = worker.clone();
    let pump_mirror = Arc::clone(&mirror);
    let pump = Closure::<dyn FnMut()>::new(move || {
        pump_commands(&pump_worker, &pump_mirror);
    });
    let Some(window) = web_sys::window() else {
        worker.terminate();
        return Err(host_error(
            ArtifactRepositoryOpenOperation::StartCommandPump,
            "browser window is unavailable",
        ));
    };
    let interval_id = match window.set_interval_with_callback_and_timeout_and_arguments_0(
        pump.as_ref().unchecked_ref(),
        PUMP_INTERVAL_MS,
    ) {
        Ok(interval_id) => interval_id,
        Err(error) => {
            worker.terminate();
            return Err(host_js_error(
                ArtifactRepositoryOpenOperation::StartCommandPump,
                error,
            ));
        }
    };
    OPFS_RUNTIMES.with(|runtimes| {
        runtimes.borrow_mut().push(OpfsRuntime {
            _worker: worker,
            _message_handler: message_handler,
            _error_handler: error_handler,
            _pump: pump,
            _interval_id: interval_id,
        });
    });
    Ok(())
}

fn pump_commands(worker: &Worker, mirror: &Arc<Mutex<MirrorState>>) {
    for _ in 0..MAX_IN_FLIGHT_COMMANDS {
        let command = {
            let Ok(mut mirror) = mirror.lock() else {
                return;
            };
            if mirror.in_flight >= MAX_IN_FLIGHT_COMMANDS {
                return;
            }
            let Some(command) = mirror.commands.pop_front() else {
                return;
            };
            mirror.queued_bytes = mirror.queued_bytes.saturating_sub(command.byte_count());
            mirror.in_flight += 1;
            command
        };
        if let Err(error) = post_command(worker, command) {
            if let Ok(mut mirror) = mirror.lock() {
                mirror.in_flight = mirror.in_flight.saturating_sub(1);
                mirror.durable = false;
            }
            tracing::warn!(%error, "could not submit browser persistence command");
        }
    }
}

impl PersistenceCommand {
    fn sequence(&self) -> u64 {
        match self {
            Self::Publish { sequence, .. } | Self::Remove { sequence, .. } => *sequence,
        }
    }

    fn key(&self) -> &ArtifactKey {
        match self {
            Self::Publish { key, .. } | Self::Remove { key, .. } => key,
        }
    }

    fn byte_count(&self) -> u64 {
        match self {
            Self::Publish { bytes, .. } => bytes.len() as u64,
            Self::Remove { .. } => 0,
        }
    }
}

fn post_command(
    worker: &Worker,
    command: PersistenceCommand,
) -> Result<(), BrowserPersistenceCommandError> {
    let kind = match command {
        PersistenceCommand::Publish { .. } => "publish",
        PersistenceCommand::Remove { .. } => "remove",
    };
    let message = message_object(kind)?;
    set(
        &message,
        "sequence",
        JsValue::from_str(&command.sequence().to_string()),
    )?;
    set(
        &message,
        "namespace",
        JsValue::from_str(command.key().namespace().as_str()),
    )?;
    set(
        &message,
        "identity",
        JsValue::from_str(&encode_identity(command.key().identity())),
    )?;
    match command {
        PersistenceCommand::Publish { bytes, .. } => {
            let array = Uint8Array::from(bytes.as_slice());
            let buffer = array.buffer();
            set(&message, "bytes", buffer.clone().into())?;
            let transfer = Array::new();
            transfer.push(&buffer);
            worker
                .post_message_with_transfer(&message, &transfer)
                .map_err(|error| BrowserPersistenceCommandError::Submission {
                    operation: BrowserPersistenceOperation::Publish,
                    detail: js_detail(error),
                })
        }
        PersistenceCommand::Remove { .. } => worker.post_message(&message).map_err(|error| {
            BrowserPersistenceCommandError::Submission {
                operation: BrowserPersistenceOperation::Remove,
                detail: js_detail(error),
            }
        }),
    }
}

fn handle_worker_message(mirror: &Arc<Mutex<MirrorState>>, value: JsValue) {
    let kind = match string_property(&value, "kind") {
        Ok(kind) => kind,
        Err(error) => {
            if let Ok(mut mirror) = mirror.lock() {
                mirror.durable = false;
                mirror.in_flight = 0;
            }
            tracing::warn!(%error, "browser persistence worker returned an invalid message");
            return;
        }
    };
    let Ok(mut mirror) = mirror.lock() else {
        return;
    };
    match kind.as_str() {
        "complete" => {
            mirror.in_flight = mirror.in_flight.saturating_sub(1);
            match completion_usage(&value) {
                Ok((quota, usage)) => {
                    mirror.quota = quota;
                    mirror.usage = usage;
                }
                Err(error) => {
                    mirror.durable = false;
                    drop(mirror);
                    tracing::warn!(%error, "browser persistence worker returned invalid completion metadata");
                }
            }
        }
        "failed" => {
            mirror.in_flight = mirror.in_flight.saturating_sub(1);
            match failure_message(&value) {
                Ok((code, message)) => {
                    if code == "site_data_lost" || code == "permission" {
                        mirror.durable = false;
                    }
                    drop(mirror);
                    tracing::warn!(%code, %message, "browser persistence command failed");
                }
                Err(error) => {
                    mirror.durable = false;
                    drop(mirror);
                    tracing::warn!(%error, "browser persistence worker returned an invalid failure message");
                }
            }
        }
        _ => {
            mirror.durable = false;
            drop(mirror);
            tracing::warn!(%kind, "browser persistence worker returned an unknown message");
        }
    }
}

fn completion_usage(
    value: &JsValue,
) -> Result<(Option<u64>, Option<u64>), BrowserArtifactProtocolError> {
    Ok((
        optional_integer_property(value, "quota")?,
        optional_integer_property(value, "usage")?,
    ))
}

fn failure_message(value: &JsValue) -> Result<(String, String), BrowserArtifactProtocolError> {
    Ok((
        string_property(value, "code")?,
        string_property(value, "message")?,
    ))
}

fn create_worker_url() -> Result<String, ArtifactRepositoryOpenError> {
    let parts = Array::new();
    parts.push(&JsValue::from_str(OPFS_WORKER_BOOTSTRAP));
    let options = BlobPropertyBag::new();
    options.set_type("text/javascript");
    let blob = Blob::new_with_str_sequence_and_options(&parts, &options).map_err(|error| {
        host_js_error(ArtifactRepositoryOpenOperation::CreateWorkerPayload, error)
    })?;
    Url::create_object_url_with_blob(&blob)
        .map_err(|error| host_js_error(ArtifactRepositoryOpenOperation::CreateWorkerUrl, error))
}

fn create_worker(worker_url: &str) -> Result<Worker, ArtifactRepositoryOpenError> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    Worker::new_with_options(worker_url, &options)
        .map_err(|error| host_js_error(ArtifactRepositoryOpenOperation::StartWorker, error))
}

fn message_object(kind: &str) -> Result<JsValue, BrowserArtifactProtocolError> {
    let message: JsValue = Object::new().into();
    set(&message, "kind", JsValue::from_str(kind))?;
    Ok(message)
}

fn property(object: &JsValue, name: &str) -> Result<JsValue, BrowserArtifactProtocolError> {
    Reflect::get(object, &JsValue::from_str(name)).map_err(|error| {
        BrowserArtifactProtocolError::PropertyAccess {
            property: name.to_owned(),
            detail: js_detail(error),
        }
    })
}

fn set(object: &JsValue, name: &str, value: JsValue) -> Result<(), BrowserArtifactProtocolError> {
    Reflect::set(object, &JsValue::from_str(name), &value)
        .map(|_| ())
        .map_err(|error| BrowserArtifactProtocolError::PropertyWrite {
            property: name.to_owned(),
            detail: js_detail(error),
        })
}

fn string_property(object: &JsValue, name: &str) -> Result<String, BrowserArtifactProtocolError> {
    property(object, name)?.as_string().ok_or_else(|| {
        BrowserArtifactProtocolError::InvalidProperty {
            property: name.to_owned(),
            expected: "a string",
        }
    })
}

fn bool_property(object: &JsValue, name: &str) -> Result<bool, BrowserArtifactProtocolError> {
    property(object, name)?
        .as_bool()
        .ok_or_else(|| BrowserArtifactProtocolError::InvalidProperty {
            property: name.to_owned(),
            expected: "a boolean",
        })
}

fn integer_property(object: &JsValue, name: &str) -> Result<u64, BrowserArtifactProtocolError> {
    let value = property(object, name)?;
    if let Some(value) = value.as_string() {
        return value
            .parse()
            .map_err(|_| BrowserArtifactProtocolError::InvalidProperty {
                property: name.to_owned(),
                expected: "an unsigned integer",
            });
    }
    value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0 && value.fract() == 0.0)
        .map(|value| value as u64)
        .ok_or_else(|| BrowserArtifactProtocolError::InvalidProperty {
            property: name.to_owned(),
            expected: "an unsigned integer",
        })
}

fn optional_integer_property(
    object: &JsValue,
    name: &str,
) -> Result<Option<u64>, BrowserArtifactProtocolError> {
    let value = property(object, name)?;
    if value.is_null() || value.is_undefined() {
        Ok(None)
    } else {
        integer_property(object, name).map(Some)
    }
}

fn encode_identity(identity: SourceIdentity) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in identity.as_bytes() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_identity(value: &str) -> Result<[u8; 32], BrowserArtifactProtocolError> {
    if value.len() != 64 {
        return Err(BrowserArtifactProtocolError::IdentityLength);
    }
    let mut identity = [0_u8; 32];
    for (index, byte) in identity.iter_mut().enumerate() {
        let offset = index * 2;
        let high = hex_value(value.as_bytes()[offset])?;
        let low = hex_value(value.as_bytes()[offset + 1])?;
        *byte = (high << 4) | low;
    }
    Ok(identity)
}

fn hex_value(value: u8) -> Result<u8, BrowserArtifactProtocolError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(BrowserArtifactProtocolError::IdentityHex),
    }
}

fn js_detail(error: JsValue) -> String {
    error.as_string().unwrap_or_else(|| format!("{error:?}"))
}

fn host_js_error(
    operation: ArtifactRepositoryOpenOperation,
    error: JsValue,
) -> ArtifactRepositoryOpenError {
    host_error(operation, js_detail(error))
}

fn host_error(
    operation: ArtifactRepositoryOpenOperation,
    message: impl Into<String>,
) -> ArtifactRepositoryOpenError {
    ArtifactRepositoryOpenError::Host {
        operation,
        message: message.into(),
    }
}

fn hydration_error(source: RepositoryError) -> ArtifactRepositoryOpenError {
    ArtifactRepositoryOpenError::Hydration { source }
}

fn protocol_error(
    source: impl std::error::Error + Send + Sync + 'static,
) -> ArtifactRepositoryOpenError {
    ArtifactRepositoryOpenError::Protocol {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod browser_repository_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn published_artifact_rehydrates_from_a_second_opfs_worker() {
        let repository = BrowserArtifactRepository::open_with_budget(
            "platform-artifact-repository-test",
            1024 * 1024,
        )
        .await
        .expect("browser test requires OPFS");
        let key = ArtifactKey::new(
            ArtifactNamespace::new("browser-opfs-test").unwrap(),
            SourceIdentity::from_bytes([0xb7; 32]),
        );
        repository.remove(&key).unwrap();
        wait_until_idle(&repository).await;

        let mut writer = repository.begin_write(key.clone()).unwrap();
        writer.write_at(0, b"persistent browser cache").unwrap();
        writer.publish().unwrap();
        wait_until_idle(&repository).await;

        let reopened = BrowserArtifactRepository::open_with_budget(
            "platform-artifact-repository-test",
            1024 * 1024,
        )
        .await
        .expect("the second worker must reopen OPFS");
        let mut reader = reopened.open(&key).unwrap().unwrap();
        let mut bytes = vec![0_u8; usize::try_from(reader.len().unwrap()).unwrap()];
        reader.read_at(0, &mut bytes).unwrap();
        assert_eq!(bytes, b"persistent browser cache");

        reopened.remove(&key).unwrap();
        wait_until_idle(&reopened).await;
    }

    async fn wait_until_idle(repository: &BrowserArtifactRepository) {
        for _ in 0..1_000 {
            if repository.idle() {
                return;
            }
            delay(10).await;
        }
        panic!("browser persistence worker did not drain its queue");
    }

    async fn delay(milliseconds: i32) {
        let promise = Promise::new(&mut |resolve, _reject| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds)
                .unwrap();
        });
        JsFuture::from(promise).await.unwrap();
    }
}
