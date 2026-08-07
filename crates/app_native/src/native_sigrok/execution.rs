use std::sync::Arc;
use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use logic_analyzer_protocol_decoders::sigrok_decoder::{
    LogicChunk, OutputRegistration, SigrokExecution, SigrokExecutionConfig, SigrokExecutionFactory,
    SigrokExecutionInput, SigrokExecutionOptionValue, SigrokExecutionOutput,
};
use logic_analyzer_protocol_decoders::types::{ProtocolPacket, ProtocolValue};
use platform_runtime::{InlineWorkExecutor, WorkExecutor};
use signal_runtime::NodeCancellation;

use super::{DecoderOutput, DecoderWorker, OptionValue, WorkerConfig, WorkerInputConfig};

const VALUE_RECURSION_LIMIT: usize = 64;

pub(crate) struct PythonSigrokExecutionFactory {
    work_executor: Arc<dyn WorkExecutor>,
}

impl PythonSigrokExecutionFactory {
    pub(crate) fn new(work_executor: Arc<dyn WorkExecutor>) -> Self {
        Self { work_executor }
    }
}

impl Default for PythonSigrokExecutionFactory {
    fn default() -> Self {
        Self::new(Arc::new(InlineWorkExecutor))
    }
}

impl SigrokExecutionFactory for PythonSigrokExecutionFactory {
    fn spawn(&self, config: SigrokExecutionConfig) -> Result<Box<dyn SigrokExecution>, String> {
        Python::initialize();
        let input = match config.input {
            SigrokExecutionInput::Logic(channels) => WorkerInputConfig::Logic(channels),
            SigrokExecutionInput::Protocol(protocols) => WorkerInputConfig::Protocol(protocols),
        };
        let options = config
            .options
            .into_iter()
            .map(|(name, value)| {
                let value = match value {
                    SigrokExecutionOptionValue::Bool(value) => OptionValue::Bool(value),
                    SigrokExecutionOptionValue::Integer(value) => OptionValue::Integer(value),
                    SigrokExecutionOptionValue::Float(value) => OptionValue::Float(value),
                    SigrokExecutionOptionValue::String(value) => OptionValue::String(value),
                };
                (name, value)
            })
            .collect();
        DecoderWorker::spawn(
            WorkerConfig {
                decoder_root: config.decoder_root,
                decoder_id: config.decoder_id,
                sample_rate: config.sample_rate,
                input,
                options,
                queue_capacity: config.queue_capacity,
            },
            Arc::clone(&self.work_executor),
        )
        .map(|worker| Box::new(PythonSigrokExecution { worker }) as Box<dyn SigrokExecution>)
        .map_err(|error| error.to_string())
    }
}

struct PythonSigrokExecution {
    worker: DecoderWorker,
}

impl SigrokExecution for PythonSigrokExecution {
    fn push_chunk(&self, chunk: LogicChunk) -> Result<(), String> {
        self.worker
            .push_chunk(chunk)
            .map_err(|error| error.to_string())
    }

    fn push_protocol_packet(&self, packet: ProtocolPacket) -> Result<(), String> {
        let value = Python::attach(|py| protocol_value_to_python(py, &packet.value, 0))
            .map_err(|error| format!("could not reconstruct Sigrok protocol packet: {error}"))?;
        self.worker
            .push_protocol_packet(
                packet.start_sample,
                packet.end_sample,
                packet.protocol_id,
                value,
            )
            .map_err(|error| error.to_string())
    }

    fn finish(&self) -> Result<(), String> {
        self.worker.finish().map_err(|error| error.to_string())
    }

    fn cancellation(&self) -> Arc<dyn NodeCancellation> {
        self.worker.cancellation()
    }

    fn try_output(&self) -> Result<Option<SigrokExecutionOutput>, String> {
        self.worker
            .try_output()
            .map_err(|error| error.to_string())?
            .map(convert_output)
            .transpose()
    }

    fn registrations(&self) -> Vec<OutputRegistration> {
        self.worker.registrations()
    }

    fn is_finished(&self) -> bool {
        self.worker.is_finished()
    }

    fn receive_output(&self, timeout: Duration) -> Result<Option<SigrokExecutionOutput>, String> {
        self.worker
            .receive_output(timeout)
            .map_err(|error| error.to_string())?
            .map(convert_output)
            .transpose()
    }

    fn join(&mut self) -> Result<(), String> {
        self.worker.join().map_err(|error| error.to_string())
    }
}

fn convert_output(output: DecoderOutput) -> Result<SigrokExecutionOutput, String> {
    let data = Python::attach(|py| python_to_protocol_value(output.data.bind(py), 0))
        .map_err(|error| format!("invalid Sigrok decoder output: {error}"))?;
    Ok(SigrokExecutionOutput {
        start_sample: output.start_sample,
        end_sample: output.end_sample,
        output_id: output.output_id,
        data,
    })
}

fn protocol_value_to_python(
    py: Python<'_>,
    value: &ProtocolValue,
    depth: usize,
) -> PyResult<Py<PyAny>> {
    if depth >= VALUE_RECURSION_LIMIT {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "protocol packet nesting exceeds 64 levels",
        ));
    }
    let value = match value {
        ProtocolValue::Null => return Ok(py.None()),
        ProtocolValue::Bool(value) => PyBool::new(py, *value).to_owned().into_any(),
        ProtocolValue::Integer(value) => PyInt::new(py, *value).into_any(),
        ProtocolValue::Float(value) => PyFloat::new(py, *value).into_any(),
        ProtocolValue::String(value) => PyString::new(py, value).into_any(),
        ProtocolValue::Bytes(value) => PyBytes::new(py, value).into_any(),
        ProtocolValue::List(values) => {
            let result = PyList::empty(py);
            for value in values {
                result.append(protocol_value_to_python(py, value, depth + 1)?)?;
            }
            result.into_any()
        }
        ProtocolValue::Tuple(values) => {
            let values = values
                .iter()
                .map(|value| protocol_value_to_python(py, value, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            PyTuple::new(py, values)?.into_any()
        }
        ProtocolValue::Mapping(values) => {
            let result = PyDict::new(py);
            for (key, value) in values {
                result.set_item(key, protocol_value_to_python(py, value, depth + 1)?)?;
            }
            result.into_any()
        }
    };
    Ok(value.unbind())
}

fn python_to_protocol_value(value: &Bound<'_, PyAny>, depth: usize) -> PyResult<ProtocolValue> {
    if depth >= VALUE_RECURSION_LIMIT {
        return Err(pyo3::exceptions::PyValueError::new_err(
            "protocol packet nesting exceeds 64 levels",
        ));
    }
    if value.is_none() {
        Ok(ProtocolValue::Null)
    } else if value.is_instance_of::<PyBool>() {
        Ok(ProtocolValue::Bool(value.extract()?))
    } else if value.is_instance_of::<PyInt>() {
        Ok(ProtocolValue::Integer(value.extract()?))
    } else if value.is_instance_of::<PyFloat>() {
        Ok(ProtocolValue::Float(value.extract()?))
    } else if value.is_instance_of::<PyString>() {
        Ok(ProtocolValue::String(value.extract()?))
    } else if let Ok(value) = value.cast::<PyBytes>() {
        Ok(ProtocolValue::Bytes(value.as_bytes().into()))
    } else if let Ok(value) = value.cast::<PyList>() {
        Ok(ProtocolValue::List(
            value
                .iter()
                .map(|item| python_to_protocol_value(&item, depth + 1))
                .collect::<PyResult<_>>()?,
        ))
    } else if let Ok(value) = value.cast::<PyTuple>() {
        Ok(ProtocolValue::Tuple(
            value
                .iter()
                .map(|item| python_to_protocol_value(&item, depth + 1))
                .collect::<PyResult<_>>()?,
        ))
    } else if let Ok(value) = value.cast::<PyDict>() {
        Ok(ProtocolValue::Mapping(
            value
                .iter()
                .map(|(key, value)| {
                    Ok((key.extract()?, python_to_protocol_value(&value, depth + 1)?))
                })
                .collect::<PyResult<_>>()?,
        ))
    } else {
        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unsupported protocol packet value {}",
            value.get_type().name()?
        )))
    }
}

#[cfg(test)]
mod execution_tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::Arc;
    use std::thread::JoinHandle;
    use std::time::Duration;

    use super::*;

    struct TestWorkExecutor;

    impl WorkExecutor for TestWorkExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn submit(
            &self,
            task: platform_runtime::WorkExecutorTask,
        ) -> Result<Box<dyn platform_runtime::WorkTask>, String> {
            self.submit_long_running(task)
        }

        fn submit_long_running(
            &self,
            task: platform_runtime::WorkExecutorTask,
        ) -> Result<Box<dyn platform_runtime::WorkTask>, String> {
            Ok(Box::new(TestWorkTask {
                handle: Some(std::thread::spawn(task)),
            }))
        }
    }

    struct TestWorkTask {
        handle: Option<JoinHandle<()>>,
    }

    impl platform_runtime::WorkTask for TestWorkTask {
        fn is_finished(&self) -> bool {
            self.handle.as_ref().is_none_or(JoinHandle::is_finished)
        }

        fn wait(mut self: Box<Self>) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    #[test]
    fn python_adapter_round_trips_owned_protocol_values() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("stacked_fixture");
        fs::create_dir(&package).unwrap();
        fs::write(package.join("__init__.py"), "from .pd import Decoder\n").unwrap();
        fs::write(
            package.join("pd.py"),
            r#"
import sigrokdecode as srd

class Decoder(srd.Decoder):
    def start(self):
        self.output = self.register(srd.OUTPUT_PYTHON, proto_id='fixture')
    def decode(self, ss, es, data):
        kind, number, details = data
        assert kind == 'DATA'
        assert number == 165
        assert details == {'valid': True, 'bytes': b'\x10\x20'}
        for _ in range(3):
            self.put(ss, es, self.output, data)
"#,
        )
        .unwrap();
        let value = ProtocolValue::Tuple(vec![
            ProtocolValue::String("DATA".into()),
            ProtocolValue::Integer(165),
            ProtocolValue::Mapping(BTreeMap::from([
                ("valid".into(), ProtocolValue::Bool(true)),
                (
                    "bytes".into(),
                    ProtocolValue::Bytes(Arc::from([0x10, 0x20])),
                ),
            ])),
        ]);
        let mut execution = PythonSigrokExecutionFactory::new(Arc::new(TestWorkExecutor))
            .spawn(SigrokExecutionConfig {
                decoder_root: directory.path().to_owned(),
                decoder_id: "stacked_fixture".into(),
                sample_rate: 1_000_000,
                input: SigrokExecutionInput::Protocol(vec!["spi".into()]),
                options: BTreeMap::new(),
                queue_capacity: 1,
            })
            .unwrap();

        execution
            .push_protocol_packet(ProtocolPacket {
                start_sample: 12,
                end_sample: 20,
                start_time_ns: 12_000,
                end_time_ns: 20_000,
                protocol_id: "spi".into(),
                value: value.clone(),
            })
            .unwrap();
        for _ in 0..3 {
            let output = execution
                .receive_output(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(output.start_sample, 12);
            assert_eq!(output.end_sample, 20);
            assert_eq!(output.data, value);
        }
        execution.finish().unwrap();
        execution.join().unwrap();
    }
}
