use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, unbounded};
use pyo3::exceptions::PyEOFError;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict, PyDictMethods, PyList, PyModule};
use thiserror::Error;

use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    InitialPin, LogicChunk, OutputRegistration,
};
use platform_runtime::{WorkExecutor, WorkTask};
use signal_runtime::NodeCancellation;

use super::bridge::{BridgeError, DecoderBridge, DecoderOutput};
use super::python_error::format_python_error;
use super::python_host::{
    HostDecoder, SRD_CONF_SAMPLERATE, decoder_import_guard, install_sigrokdecode_module,
};

#[derive(Clone, Debug)]
pub(crate) enum OptionValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug)]
pub(crate) struct WorkerConfig {
    pub(crate) decoder_root: PathBuf,
    pub(crate) decoder_id: String,
    pub(crate) sample_rate: u64,
    pub(crate) input: WorkerInputConfig,
    pub(crate) options: BTreeMap<String, OptionValue>,
    pub(crate) queue_capacity: usize,
}

#[derive(Clone, Debug)]
pub(crate) enum WorkerInputConfig {
    Logic(Vec<Option<InitialPin>>),
    Protocol(Vec<String>),
}

enum ProtocolInputMessage {
    Packet {
        start_sample: u64,
        end_sample: u64,
        protocol_id: String,
        value: Py<PyAny>,
    },
    Finish,
    Cancel,
}

#[derive(Debug, Error)]
pub(crate) enum WorkerError {
    #[error(transparent)]
    Bridge(#[from] BridgeError),
    #[error("failed to start Sigrok decoder worker: {0}")]
    Execution(String),
    #[error("Sigrok decoder worker result was unavailable")]
    ResultUnavailable,
    #[error("Sigrok decoder failed:\n{0}")]
    Python(String),
}

pub(crate) struct DecoderWorker {
    bridge: Arc<DecoderBridge>,
    outputs: Receiver<DecoderOutput>,
    task: Option<Box<dyn WorkTask>>,
    result: Receiver<Result<(), WorkerError>>,
    cancellation: Arc<DecoderCancellation>,
}

pub(crate) struct DecoderCancellation {
    bridge: Arc<DecoderBridge>,
    protocol_input: Option<Sender<ProtocolInputMessage>>,
}

impl NodeCancellation for DecoderCancellation {
    fn request_cancel(&self) {
        if let Some(sender) = &self.protocol_input {
            let _ = sender.send(ProtocolInputMessage::Cancel);
        } else {
            self.bridge.cancel();
        }
    }
}

impl DecoderWorker {
    pub(crate) fn spawn(
        config: WorkerConfig,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<Self, WorkerError> {
        let (bridge, outputs) = match &config.input {
            WorkerInputConfig::Logic(channels) => {
                DecoderBridge::new(channels.clone(), config.queue_capacity)?
            }
            WorkerInputConfig::Protocol(_) => DecoderBridge::new_protocol(config.queue_capacity),
        };
        let (protocol_sender, protocol_receiver) = unbounded();
        let protocol_input =
            matches!(&config.input, WorkerInputConfig::Protocol(_)).then_some(protocol_sender);
        let cancellation = Arc::new(DecoderCancellation {
            bridge: Arc::clone(&bridge),
            protocol_input,
        });
        let task_bridge = Arc::clone(&bridge);
        let (result_sender, result) = crossbeam_channel::bounded(1);
        let task = work_executor
            .submit_long_running(Box::new(move || {
                let _ = result_sender.send(run_decoder(config, task_bridge, protocol_receiver));
            }))
            .map_err(WorkerError::Execution)?;
        Ok(Self {
            bridge,
            outputs,
            task: Some(task),
            result,
            cancellation,
        })
    }

    pub(crate) fn push_chunk(&self, chunk: LogicChunk) -> Result<(), WorkerError> {
        self.bridge.push_chunk(chunk).map_err(Into::into)
    }

    pub(crate) fn push_protocol_packet(
        &self,
        start_sample: u64,
        end_sample: u64,
        protocol_id: String,
        value: Py<PyAny>,
    ) -> Result<(), WorkerError> {
        let sender = self
            .cancellation
            .protocol_input
            .as_ref()
            .ok_or(BridgeError::ProtocolInputUnavailable)?;
        sender
            .send(ProtocolInputMessage::Packet {
                start_sample,
                end_sample,
                protocol_id,
                value,
            })
            .map_err(|_| BridgeError::InputQueueClosed.into())
    }

    pub(crate) fn finish(&self) -> Result<(), WorkerError> {
        if let Some(sender) = &self.cancellation.protocol_input {
            sender
                .send(ProtocolInputMessage::Finish)
                .map_err(|_| BridgeError::InputQueueClosed.into())
        } else {
            self.bridge.finish().map_err(Into::into)
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancellation.request_cancel();
    }

    pub(crate) fn cancellation(&self) -> Arc<dyn NodeCancellation> {
        self.cancellation.clone()
    }

    pub(crate) fn try_output(&self) -> Result<Option<DecoderOutput>, WorkerError> {
        match self.outputs.try_recv() {
            Ok(output) => Ok(Some(output)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Ok(None),
        }
    }

    pub(crate) fn registrations(&self) -> Vec<OutputRegistration> {
        self.bridge.registrations()
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.task.as_ref().is_none_or(|task| task.is_finished())
    }

    pub(crate) fn receive_output(
        &self,
        timeout: std::time::Duration,
    ) -> Result<Option<DecoderOutput>, WorkerError> {
        match self.outputs.recv_timeout(timeout) {
            Ok(output) => Ok(Some(output)),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    pub(crate) fn join(&mut self) -> Result<(), WorkerError> {
        self.join_inner()
    }

    fn join_inner(&mut self) -> Result<(), WorkerError> {
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.wait();
        self.result
            .recv()
            .map_err(|_| WorkerError::ResultUnavailable)?
    }
}

impl Drop for DecoderWorker {
    fn drop(&mut self) {
        if self.task.is_some() {
            self.cancel();
            let _ = self.join_inner();
        }
    }
}

fn run_decoder(
    config: WorkerConfig,
    bridge: Arc<DecoderBridge>,
    protocol_input: Receiver<ProtocolInputMessage>,
) -> Result<(), WorkerError> {
    Python::initialize();
    let import_guard = decoder_import_guard();
    Python::attach(move |py| {
        let decoder = {
            install_sigrokdecode_module(py)?;
            let decoder_class = import_decoder(py, &config.decoder_root, &config.decoder_id)?;
            decoder_class.call0()?
        };
        drop(import_guard);
        decoder.cast::<HostDecoder>()?.borrow_mut().attach(bridge);

        let options = PyDict::new(py);
        for (name, value) in &config.options {
            match value {
                OptionValue::Bool(value) => options.set_item(name, *value)?,
                OptionValue::Integer(value) => options.set_item(name, *value)?,
                OptionValue::Float(value) => options.set_item(name, *value)?,
                OptionValue::String(value) => options.set_item(name, value)?,
            }
        }
        decoder.setattr("options", options)?;
        decoder.setattr("samplenum", 0)?;
        decoder.setattr("matched", py.None())?;
        if decoder.hasattr("metadata")? {
            decoder.call_method1("metadata", (SRD_CONF_SAMPLERATE, config.sample_rate))?;
        }
        decoder.call_method0("start")?;
        match &config.input {
            WorkerInputConfig::Logic(_) => match decoder.call_method0("decode") {
                Ok(_) => Ok(()),
                Err(error) if error.is_instance_of::<PyEOFError>(py) => Ok(()),
                Err(error) => Err(error),
            },
            WorkerInputConfig::Protocol(accepted_protocols) => loop {
                let message = py.detach(|| protocol_input.recv()).map_err(|_| {
                    pyo3::exceptions::PyRuntimeError::new_err("protocol input closed")
                })?;
                match message {
                    ProtocolInputMessage::Packet {
                        start_sample,
                        end_sample,
                        protocol_id,
                        value,
                    } => {
                        if !accepted_protocols.contains(&protocol_id) {
                            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                                "decoder '{}' does not accept protocol '{protocol_id}'",
                                config.decoder_id
                            )));
                        }
                        decoder
                            .call_method1("decode", (start_sample, end_sample, value.bind(py)))?;
                    }
                    ProtocolInputMessage::Finish | ProtocolInputMessage::Cancel => return Ok(()),
                }
            },
        }
    })
    .map_err(|error| WorkerError::Python(format_python_error(error)))
}

fn import_decoder<'py>(
    py: Python<'py>,
    decoder_root: &Path,
    decoder_id: &str,
) -> PyResult<Bound<'py, PyAny>> {
    let decoder_root = decoder_root
        .to_str()
        .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("decoder path is not UTF-8"))?;
    let sys = PyModule::import(py, "sys")?;
    let path: Bound<'_, PyList> = sys.getattr("path")?.cast_into()?;
    path.call_method1("insert", (0, decoder_root))?;
    let modules: Bound<'_, PyDict> = sys.getattr("modules")?.cast_into()?;
    modules.del_item(decoder_id).ok();
    modules.del_item(format!("{decoder_id}.pd")).ok();
    PyModule::import(py, decoder_id)?.getattr("Decoder")
}
