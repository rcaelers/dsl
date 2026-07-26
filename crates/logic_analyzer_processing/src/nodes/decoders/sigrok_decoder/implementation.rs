use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};

use signal_processing::{
    InputPort, NodeCancellation, OutputPort, PortDirection, PortSchema, ProcessNode,
    ProtocolPacket, ProtocolValue, SampleBlock, Word, WorkError, WorkResult,
};

use crate::support::sigrokdecode::{
    DecoderOutput, DecoderWorker, InitialPin, LogicChunk, MetadataType, OUTPUT_ANN, OUTPUT_BINARY,
    OUTPUT_LOGIC, OUTPUT_META, OUTPUT_PYTHON, OptionValue, OutputRegistration, WorkerConfig,
    WorkerError, WorkerInputConfig,
};

const OUTPUT_QUEUE_CAPACITY: usize = 65_536;
const OUTPUT_WAIT: Duration = Duration::from_millis(2);
const VALUE_RECURSION_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigrokInitialPin {
    Low,
    High,
    SameAsFirstSample,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokChannel {
    pub name: String,
    pub connected: bool,
    pub initial_pin: SigrokInitialPin,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SigrokOptionValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SigrokDecoderConfig {
    pub decoder_root: PathBuf,
    pub decoder_id: String,
    pub sample_rate: u64,
    pub channels: Vec<SigrokChannel>,
    pub protocol_inputs: Vec<String>,
    pub options: BTreeMap<String, SigrokOptionValue>,
    pub annotation_rows_by_class: Vec<Arc<[usize]>>,
    pub binary_class_count: usize,
    pub logic_groups: Vec<String>,
}

pub struct SigrokDecoder {
    name: String,
    decoder_id: String,
    sample_rate: u64,
    input_timestamp_step_ns: Option<u64>,
    channels: Vec<SigrokChannel>,
    annotation_rows_by_class: Vec<Arc<[usize]>>,
    binary_class_count: usize,
    logic_groups: Vec<String>,
    input_buffers: Vec<VecDeque<SampleBlock>>,
    protocol_inputs: Vec<String>,
    protocol_buffer: VecDeque<ProtocolPacket>,
    worker: DecoderWorker,
    finished: bool,
}

impl SigrokDecoder {
    pub fn new(config: SigrokDecoderConfig) -> Result<Self, String> {
        Python::initialize();
        if config.sample_rate == 0 {
            return Err("Sigrok decoder sample rate must be positive".into());
        }
        let connected_count = config
            .channels
            .iter()
            .filter(|channel| channel.connected)
            .count();
        match (connected_count > 0, config.protocol_inputs.is_empty()) {
            (false, true) => {
                return Err("Sigrok decoder requires raw-logic or protocol input".into());
            }
            (true, false) => {
                return Err(
                    "Sigrok decoder cannot mix raw-logic channels and protocol input".into(),
                );
            }
            _ => {}
        }
        let channel_initial = config
            .channels
            .iter()
            .map(|channel| {
                channel.connected.then_some(match channel.initial_pin {
                    SigrokInitialPin::Low => InitialPin::Low,
                    SigrokInitialPin::High => InitialPin::High,
                    SigrokInitialPin::SameAsFirstSample => InitialPin::SameAsFirstSample,
                })
            })
            .collect();
        let options = config
            .options
            .iter()
            .map(|(name, value)| {
                let value = match value {
                    SigrokOptionValue::Bool(value) => OptionValue::Bool(*value),
                    SigrokOptionValue::Integer(value) => OptionValue::Integer(*value),
                    SigrokOptionValue::Float(value) => OptionValue::Float(*value),
                    SigrokOptionValue::String(value) => OptionValue::String(value.clone()),
                };
                (name.clone(), value)
            })
            .collect();
        let worker_input = if config.protocol_inputs.is_empty() {
            WorkerInputConfig::Logic(channel_initial)
        } else {
            WorkerInputConfig::Protocol(config.protocol_inputs.clone())
        };
        let worker = DecoderWorker::spawn(WorkerConfig {
            decoder_root: config.decoder_root.clone(),
            decoder_id: config.decoder_id.clone(),
            sample_rate: config.sample_rate,
            input: worker_input,
            options,
            queue_capacity: OUTPUT_QUEUE_CAPACITY,
        })
        .map_err(|error| error.to_string())?;
        Ok(Self {
            name: format!("sigrok_{}", config.decoder_id),
            decoder_id: config.decoder_id,
            sample_rate: config.sample_rate,
            input_timestamp_step_ns: None,
            channels: config.channels,
            annotation_rows_by_class: config.annotation_rows_by_class,
            binary_class_count: config.binary_class_count,
            logic_groups: config.logic_groups,
            input_buffers: (0..connected_count).map(|_| VecDeque::new()).collect(),
            protocol_inputs: config.protocol_inputs,
            protocol_buffer: VecDeque::new(),
            worker,
            finished: false,
        })
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    fn acquire_chunk(&mut self, inputs: &[InputPort]) -> WorkResult<Option<LogicChunk>> {
        let mut blocks = Vec::with_capacity(inputs.len());
        for (index, (input, buffer)) in inputs.iter().zip(&mut self.input_buffers).enumerate() {
            let Some(mut receiver) = input.get::<SampleBlock>(buffer) else {
                return Err(WorkError::NodeError(format!(
                    "missing Sigrok decoder input {index}"
                )));
            };
            match receiver.recv() {
                Ok(block) => blocks.push(block),
                Err(WorkError::Shutdown) if index == 0 => return Ok(None),
                Err(WorkError::Shutdown) => {
                    return Err(WorkError::NodeError(format!(
                        "Sigrok decoder input {index} ended before input 0"
                    )));
                }
                Err(error) => return Err(error),
            }
        }
        let first = blocks.first().expect("at least one connected channel");
        if first.num_samples == 0 {
            return Err(WorkError::NodeError(
                "Sigrok decoder received an empty sample block".into(),
            ));
        }
        if first.timestamp_step == 0 {
            return Err(WorkError::NodeError(
                "Sigrok decoder received a zero input timestamp step".into(),
            ));
        }
        match self.input_timestamp_step_ns {
            Some(timestamp_step) if timestamp_step != first.timestamp_step => {
                return Err(WorkError::NodeError(format!(
                    "Sigrok decoder input timestamp step changed from {timestamp_step} ns to {} ns",
                    first.timestamp_step
                )));
            }
            None => self.input_timestamp_step_ns = Some(first.timestamp_step),
            _ => {}
        }
        for (index, block) in blocks.iter().enumerate().skip(1) {
            if block.start_position != first.start_position
                || block.num_samples != first.num_samples
                || block.timestamp_step != first.timestamp_step
            {
                return Err(WorkError::NodeError(format!(
                    "Sigrok decoder input {index} is not aligned with input 0"
                )));
            }
        }
        let start_position = first.start_position;
        let sample_count = first.num_samples;
        let required_bytes = first.num_samples.div_ceil(8);
        let mut connected = blocks.into_iter();
        let channels = self
            .channels
            .iter()
            .map(|channel| {
                if !channel.connected {
                    return Ok(None);
                }
                let block = connected.next().expect("connected input count validated");
                if block.data.len() < required_bytes {
                    return Err(WorkError::NodeError(format!(
                        "Sigrok decoder input '{}' has {} bytes for {} samples",
                        channel.name,
                        block.data.len(),
                        block.num_samples
                    )));
                }
                Ok(Some(Arc::from(&block.data[..required_bytes])))
            })
            .collect::<WorkResult<Vec<_>>>()?;
        Ok(Some(LogicChunk::new(
            start_position,
            sample_count,
            channels,
        )))
    }

    fn acquire_protocol_packet(
        &mut self,
        inputs: &[InputPort],
    ) -> WorkResult<Option<ProtocolPacket>> {
        let Some(input) = inputs.first() else {
            return Err(WorkError::NodeError(
                "missing Sigrok decoder protocol input".into(),
            ));
        };
        let Some(mut receiver) = input.get::<ProtocolPacket>(&mut self.protocol_buffer) else {
            return Err(WorkError::NodeError(
                "invalid Sigrok decoder protocol input".into(),
            ));
        };
        match receiver.recv() {
            Ok(packet) => Ok(Some(packet)),
            Err(WorkError::Shutdown) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn drain_outputs(&mut self, outputs: &[OutputPort]) -> WorkResult<usize> {
        let registrations = self.worker.registrations();
        let mut converted = Vec::new();
        while let Some(output) = self.worker.try_output().map_err(worker_error)? {
            converted.push(self.convert_output(output, &registrations)?);
        }
        let count = converted.len();
        for output in converted {
            send_output(outputs, output)?;
        }
        Ok(count)
    }

    fn finalize(&mut self, outputs: &[OutputPort]) -> WorkResult<usize> {
        self.worker.finish().map_err(worker_error)?;
        let mut count = 0;
        while !self.worker.is_finished() {
            if let Some(output) = self
                .worker
                .receive_output(OUTPUT_WAIT)
                .map_err(worker_error)?
            {
                let registrations = self.worker.registrations();
                send_output(outputs, self.convert_output(output, &registrations)?)?;
                count += 1;
            }
        }
        self.worker.join().map_err(worker_error)?;
        count += self.drain_outputs(outputs)?;
        self.finished = true;
        Ok(count)
    }

    fn convert_output(
        &self,
        output: DecoderOutput,
        registrations: &[OutputRegistration],
    ) -> WorkResult<ConvertedOutput> {
        let registration = registrations.get(output.output_id).ok_or_else(|| {
            WorkError::NodeError(format!(
                "Sigrok decoder returned unknown output ID {}",
                output.output_id
            ))
        })?;
        let start_time_ns = self.sample_time_ns(output.start_sample);
        let end_time_ns = self.sample_time_ns(output.end_sample);
        Python::attach(|py| {
            convert_python_output(
                output.data.bind(py),
                registration,
                start_time_ns,
                end_time_ns,
                output.start_sample,
                output.end_sample,
                &self.decoder_id,
                &self.annotation_rows_by_class,
                self.binary_class_count,
                &self.logic_groups,
            )
        })
        .map_err(|error| WorkError::NodeError(format!("invalid Sigrok decoder output: {error}")))
    }

    fn sample_time_ns(&self, sample: u64) -> u64 {
        sample_time_ns(sample, self.input_timestamp_step_ns, self.sample_rate)
    }
}

impl ProcessNode for SigrokDecoder {
    fn name(&self) -> &str {
        &self.name
    }

    fn should_stop(&self) -> bool {
        self.finished
    }

    fn num_inputs(&self) -> usize {
        if self.protocol_inputs.is_empty() {
            self.input_buffers.len()
        } else {
            1
        }
    }

    fn num_outputs(&self) -> usize {
        5
    }

    fn input_schema(&self) -> Vec<PortSchema> {
        if !self.protocol_inputs.is_empty() {
            return vec![PortSchema::new::<ProtocolPacket>(
                "packets",
                0,
                PortDirection::Input,
            )];
        }
        self.channels
            .iter()
            .filter(|channel| channel.connected)
            .enumerate()
            .map(|(index, channel)| {
                PortSchema::new::<SampleBlock>(&channel.name, index, PortDirection::Input)
            })
            .collect()
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        vec![
            PortSchema::new::<Word>("annotations", 0, PortDirection::Output),
            PortSchema::new::<Word>("binary", 1, PortDirection::Output),
            PortSchema::new::<SampleBlock>("logic", 2, PortDirection::Output),
            PortSchema::new::<Word>("metadata", 3, PortDirection::Output),
            PortSchema::new::<ProtocolPacket>("packets", 4, PortDirection::Output),
        ]
    }

    fn cancellation(&self) -> Option<Arc<dyn NodeCancellation>> {
        Some(self.worker.cancellation())
    }

    fn work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        if self.finished {
            return Err(WorkError::Shutdown);
        }
        if self.worker.is_finished() {
            self.worker.join().map_err(worker_error)?;
            self.finished = true;
            return Err(WorkError::NodeError(
                "Sigrok decoder stopped before its inputs ended".into(),
            ));
        }
        if !self.protocol_inputs.is_empty() {
            let Some(packet) = self.acquire_protocol_packet(inputs)? else {
                let count = self.finalize(outputs)?;
                return if count == 0 {
                    Err(WorkError::Shutdown)
                } else {
                    Ok(count)
                };
            };
            let value = Python::attach(|py| sigrok_value_to_python(py, &packet.value, 0)).map_err(
                |error| {
                    WorkError::NodeError(format!(
                        "could not reconstruct Sigrok protocol packet: {error}"
                    ))
                },
            )?;
            self.worker
                .push_protocol_packet(
                    packet.start_sample,
                    packet.end_sample,
                    packet.protocol_id,
                    value,
                )
                .map_err(worker_error)?;
            return Ok(1 + self.drain_outputs(outputs)?);
        }
        let Some(chunk) = self.acquire_chunk(inputs)? else {
            let count = self.finalize(outputs)?;
            return if count == 0 {
                Err(WorkError::Shutdown)
            } else {
                Ok(count)
            };
        };
        let sample_count = chunk.sample_count();
        self.worker.push_chunk(chunk).map_err(worker_error)?;
        Ok(sample_count + self.drain_outputs(outputs)?)
    }
}

enum ConvertedOutput {
    Annotation(Word),
    Binary(Word),
    Logic(SampleBlock),
    Metadata(Word),
    Packet(ProtocolPacket),
}

fn send_output(outputs: &[OutputPort], output: ConvertedOutput) -> WorkResult<()> {
    match output {
        ConvertedOutput::Annotation(value) => {
            if let Some(sender) = outputs.first().and_then(|output| output.get()) {
                sender.send(value)?;
            }
        }
        ConvertedOutput::Binary(value) => {
            if let Some(sender) = outputs.get(1).and_then(|output| output.get()) {
                sender.send(value)?;
            }
        }
        ConvertedOutput::Logic(value) => {
            if let Some(sender) = outputs.get(2).and_then(|output| output.get()) {
                sender.send(value)?;
            }
        }
        ConvertedOutput::Metadata(value) => {
            if let Some(sender) = outputs.get(3).and_then(|output| output.get()) {
                sender.send(value)?;
            }
        }
        ConvertedOutput::Packet(value) => {
            if let Some(sender) = outputs.get(4).and_then(|output| output.get()) {
                sender.send(value)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn convert_python_output(
    data: &Bound<'_, PyAny>,
    registration: &OutputRegistration,
    start_time_ns: u64,
    end_time_ns: u64,
    start_sample: u64,
    end_sample: u64,
    decoder_id: &str,
    annotation_rows_by_class: &[Arc<[usize]>],
    binary_class_count: usize,
    logic_groups: &[String],
) -> PyResult<ConvertedOutput> {
    match registration.output_type {
        OUTPUT_ANN => {
            let data = data.cast::<PyList>()?;
            require_length(data.len(), 2, "annotation")?;
            let class: usize = data.get_item(0)?.extract()?;
            let _rows = annotation_rows_by_class
                .get(class)
                .cloned()
                .ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(format!(
                        "annotation class {class} is not declared"
                    ))
                })?;
            let texts = data
                .get_item(1)?
                .cast::<PyList>()?
                .iter()
                .map(|text| text.extract())
                .collect::<PyResult<Vec<String>>>()?;
            let label = texts.first().cloned().unwrap_or_default();
            Ok(ConvertedOutput::Annotation(Word::labeled(
                class as u64,
                label,
                start_time_ns,
                end_time_ns.saturating_sub(start_time_ns),
            )))
        }
        OUTPUT_BINARY => {
            let data = data.cast::<PyList>()?;
            require_length(data.len(), 2, "binary output")?;
            let class: usize = data.get_item(0)?.extract()?;
            if class >= binary_class_count {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "binary class {class} is not declared"
                )));
            }
            let bytes: Arc<[u8]> = data.get_item(1)?.cast::<PyBytes>()?.as_bytes().into();
            Ok(ConvertedOutput::Binary(Word::bytes_with_tag(
                class as u64,
                bytes,
                start_time_ns,
                end_time_ns.saturating_sub(start_time_ns),
            )))
        }
        OUTPUT_LOGIC => {
            let data = data.cast::<PyList>()?;
            require_length(data.len(), 2, "generated logic output")?;
            let group_index: usize = data.get_item(0)?.extract()?;
            let _group = logic_groups.get(group_index).cloned().ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "logic group {group_index} is not declared"
                ))
            })?;
            let samples: Arc<[u8]> = data.get_item(1)?.cast::<PyBytes>()?.as_bytes().into();
            let sample_count = end_sample
                .saturating_sub(start_sample)
                .saturating_sub(1)
                .min(usize::MAX as u64) as usize;
            let timestamp_step = end_time_ns
                .saturating_sub(start_time_ns)
                .checked_div(sample_count.max(1) as u64)
                .unwrap_or(1)
                .max(1);
            Ok(ConvertedOutput::Logic(SampleBlock::new(
                samples,
                start_sample,
                sample_count,
                timestamp_step,
            )))
        }
        OUTPUT_META => {
            let metadata = registration.metadata.as_ref().ok_or_else(|| {
                pyo3::exceptions::PyValueError::new_err(
                    "metadata output has no registration descriptor",
                )
            })?;
            let (value, display) = match metadata.value_type {
                MetadataType::Integer => {
                    let value: i64 = data.extract()?;
                    (value as u64, format!("{}: {value}", metadata.name))
                }
                MetadataType::Float => {
                    let value: f64 = data.extract()?;
                    (value.to_bits(), format!("{}: {value}", metadata.name))
                }
            };
            Ok(ConvertedOutput::Metadata(Word::labeled(
                value,
                display,
                start_time_ns,
                end_time_ns.saturating_sub(start_time_ns),
            )))
        }
        OUTPUT_PYTHON => Ok(ConvertedOutput::Packet(ProtocolPacket {
            start_sample,
            end_sample,
            start_time_ns,
            end_time_ns,
            protocol_id: registration
                .protocol_id
                .clone()
                .unwrap_or_else(|| decoder_id.to_owned()),
            value: convert_value(data, 0)?,
        })),
        output_type => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unsupported output type {output_type}"
        ))),
    }
}

fn sigrok_value_to_python(
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
                result.append(sigrok_value_to_python(py, value, depth + 1)?)?;
            }
            result.into_any()
        }
        ProtocolValue::Tuple(values) => {
            let values = values
                .iter()
                .map(|value| sigrok_value_to_python(py, value, depth + 1))
                .collect::<PyResult<Vec<_>>>()?;
            PyTuple::new(py, values)?.into_any()
        }
        ProtocolValue::Mapping(values) => {
            let result = PyDict::new(py);
            for (key, value) in values {
                result.set_item(key, sigrok_value_to_python(py, value, depth + 1)?)?;
            }
            result.into_any()
        }
    };
    Ok(value.unbind())
}

fn convert_value(value: &Bound<'_, PyAny>, depth: usize) -> PyResult<ProtocolValue> {
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
                .map(|item| convert_value(&item, depth + 1))
                .collect::<PyResult<_>>()?,
        ))
    } else if let Ok(value) = value.cast::<PyTuple>() {
        Ok(ProtocolValue::Tuple(
            value
                .iter()
                .map(|item| convert_value(&item, depth + 1))
                .collect::<PyResult<_>>()?,
        ))
    } else if let Ok(value) = value.cast::<PyDict>() {
        Ok(ProtocolValue::Mapping(
            value
                .iter()
                .map(|(key, value)| Ok((key.extract()?, convert_value(&value, depth + 1)?)))
                .collect::<PyResult<_>>()?,
        ))
    } else {
        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unsupported protocol packet value {}",
            value.get_type().name()?
        )))
    }
}

fn require_length(actual: usize, expected: usize, name: &str) -> PyResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(pyo3::exceptions::PyValueError::new_err(format!(
            "{name} must have {expected} elements, got {actual}"
        )))
    }
}

fn sample_time_ns(sample: u64, timestamp_step_ns: Option<u64>, sample_rate: u64) -> u64 {
    timestamp_step_ns.map_or_else(
        || {
            (u128::from(sample) * 1_000_000_000 / u128::from(sample_rate)).min(u128::from(u64::MAX))
                as u64
        },
        |timestamp_step| sample.saturating_mul(timestamp_step),
    )
}

fn worker_error(error: WorkerError) -> WorkError {
    WorkError::NodeError(error.to_string())
}

#[cfg(test)]
mod implementation_tests {
    use std::fs;

    use crossbeam_channel::{Receiver as ChannelReceiver, bounded};

    use signal_processing::{ChannelMessage, Sender, Watchdog, WordPayload};

    use super::*;

    #[test]
    fn raw_input_timing_takes_precedence_over_the_decoder_configuration() {
        assert_eq!(sample_time_ns(645_812, Some(1_000), 2_000_000), 645_812_000);
    }

    #[test]
    fn protocol_input_reconstructs_owned_values_for_stacked_decode_calls() {
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
        self.ann = self.register(srd.OUTPUT_ANN)
    def decode(self, ss, es, data):
        kind, number, details = data
        assert kind == 'DATA'
        assert number == 165
        assert details == {'valid': True, 'bytes': b'\x10\x20'}
        self.put(ss, es, self.ann, [0, [f'{kind}:{number}:{details["valid"]}']])
"#,
        )
        .unwrap();

        let watchdog = Watchdog::new();
        let (input_sender, input_receiver) = bounded(4);
        input_sender
            .send(ChannelMessage::Sample(ProtocolPacket {
                start_sample: 12,
                end_sample: 20,
                start_time_ns: 12_000,
                end_time_ns: 20_000,
                protocol_id: "spi".into(),
                value: ProtocolValue::Tuple(vec![
                    ProtocolValue::String("DATA".into()),
                    ProtocolValue::Integer(165),
                    ProtocolValue::Mapping(BTreeMap::from([
                        ("valid".into(), ProtocolValue::Bool(true)),
                        (
                            "bytes".into(),
                            ProtocolValue::Bytes(Arc::from([0x10, 0x20])),
                        ),
                    ])),
                ]),
            }))
            .unwrap();
        drop(input_sender);
        let inputs = vec![InputPort::new_with_watchdog(
            input_receiver,
            &watchdog,
            "stacked-test",
            "packets",
        )];
        let (annotation_output, annotation_receiver) = output::<Word>(&watchdog, 0);
        let (binary_output, _binary_receiver) = output::<Word>(&watchdog, 1);
        let (logic_output, _logic_receiver) = output::<SampleBlock>(&watchdog, 2);
        let (metadata_output, _metadata_receiver) = output::<Word>(&watchdog, 3);
        let (packet_output, _packet_receiver) = output::<ProtocolPacket>(&watchdog, 4);
        let outputs = vec![
            annotation_output,
            binary_output,
            logic_output,
            metadata_output,
            packet_output,
        ];
        let mut decoder = SigrokDecoder::new(SigrokDecoderConfig {
            decoder_root: directory.path().to_owned(),
            decoder_id: "stacked_fixture".into(),
            sample_rate: 1_000_000,
            channels: Vec::new(),
            protocol_inputs: vec!["spi".into()],
            options: BTreeMap::new(),
            annotation_rows_by_class: vec![Arc::from([0])],
            binary_class_count: 0,
            logic_groups: Vec::new(),
        })
        .unwrap();
        loop {
            match decoder.work(&inputs, &outputs) {
                Ok(_) if decoder.should_stop() => break,
                Ok(_) => {}
                Err(WorkError::Shutdown) => break,
                Err(error) => panic!("unexpected stacked decoder error: {error}"),
            }
        }
        let annotations = collect(annotation_receiver);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].timestamp_ns, 12_000);
        assert_eq!(annotations[0].end_ns(), 20_000);
        assert_eq!(word_text(&annotations[0]), "DATA:165:True");
    }

    fn word_text(word: &Word) -> String {
        match word.payload.as_ref() {
            Some(WordPayload::Text(text)) => text.to_string(),
            Some(WordPayload::Bytes(bytes)) => bytes
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" "),
            None => word.value.to_string(),
        }
    }

    fn output<T: Clone + Send + 'static>(
        watchdog: &Watchdog,
        index: usize,
    ) -> (OutputPort, ChannelReceiver<ChannelMessage<T>>) {
        let (sender, receiver) = bounded(1_024);
        (
            OutputPort::new_with_watchdog(
                Sender::new(vec![sender]),
                watchdog,
                "sigrok-test",
                &format!("out{index}"),
            ),
            receiver,
        )
    }

    fn collect<T>(receiver: ChannelReceiver<ChannelMessage<T>>) -> Vec<T> {
        receiver
            .try_iter()
            .flat_map(|message| match message {
                ChannelMessage::Sample(value) => vec![value],
                ChannelMessage::Batch(values) => values,
                ChannelMessage::EndOfStream => Vec::new(),
            })
            .collect()
    }
}
