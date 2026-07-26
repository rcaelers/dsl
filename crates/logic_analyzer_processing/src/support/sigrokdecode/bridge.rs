use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded, select_biased};
use pyo3::Py;
use pyo3::types::PyAny;
use thiserror::Error;

use super::conditions::WaitRequest;
use super::scheduler::{LogicChunk, SchedulerError, SchedulerStatus, WaitMatch, WaitScheduler};

#[derive(Debug)]
enum InputMessage {
    Chunk(LogicChunk),
    Finish,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputRegistration {
    pub(crate) output_type: i32,
    pub(crate) protocol_id: Option<String>,
    pub(crate) metadata: Option<MetadataRegistration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataType {
    Integer,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataRegistration {
    pub(crate) value_type: MetadataType,
    pub(crate) name: String,
    pub(crate) description: String,
}

#[derive(Debug)]
pub(crate) struct DecoderOutput {
    pub(crate) start_sample: u64,
    pub(crate) end_sample: u64,
    pub(crate) output_id: usize,
    pub(crate) data: Py<PyAny>,
}

#[derive(Debug, Error)]
pub(crate) enum BridgeError {
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error("decoder input queue is full")]
    InputQueueFull,
    #[error("decoder input queue is closed")]
    InputQueueClosed,
    #[error("decoder output queue is closed")]
    OutputQueueClosed,
    #[error("decoder was cancelled")]
    Cancelled,
    #[error("decoder output ID {0} is not registered")]
    UnknownOutput(usize),
    #[error("decoder output ends before it starts")]
    ReversedOutputRange,
    #[error("wait() is unavailable for a protocol-input decoder")]
    WaitUnavailable,
    #[error("raw logic input is unavailable for a protocol-input decoder")]
    RawInputUnavailable,
    #[error("protocol input is unavailable for a raw-logic decoder")]
    ProtocolInputUnavailable,
}

pub(crate) struct DecoderBridge {
    scheduler: Option<Mutex<WaitScheduler>>,
    input_sender: Sender<InputMessage>,
    input_receiver: Receiver<InputMessage>,
    cancel_sender: Sender<()>,
    cancel_receiver: Receiver<()>,
    output_sender: Sender<DecoderOutput>,
    registrations: Mutex<Vec<OutputRegistration>>,
    connected_channels: Vec<bool>,
}

impl DecoderBridge {
    pub(crate) fn new(
        channel_initial: Vec<Option<super::scheduler::InitialPin>>,
        queue_capacity: usize,
    ) -> Result<(Arc<Self>, Receiver<DecoderOutput>), BridgeError> {
        let connected_channels = channel_initial.iter().map(Option::is_some).collect();
        let scheduler = WaitScheduler::new(channel_initial)?;
        let (input_sender, input_receiver) = bounded(queue_capacity);
        let (cancel_sender, cancel_receiver) = bounded(1);
        let (output_sender, output_receiver) = bounded(queue_capacity);
        Ok((
            Arc::new(Self {
                scheduler: Some(Mutex::new(scheduler)),
                input_sender,
                input_receiver,
                cancel_sender,
                cancel_receiver,
                output_sender,
                registrations: Mutex::new(Vec::new()),
                connected_channels,
            }),
            output_receiver,
        ))
    }

    pub(crate) fn new_protocol(queue_capacity: usize) -> (Arc<Self>, Receiver<DecoderOutput>) {
        let (input_sender, input_receiver) = bounded(queue_capacity);
        let (cancel_sender, cancel_receiver) = bounded(1);
        let (output_sender, output_receiver) = bounded(queue_capacity);
        (
            Arc::new(Self {
                scheduler: None,
                input_sender,
                input_receiver,
                cancel_sender,
                cancel_receiver,
                output_sender,
                registrations: Mutex::new(Vec::new()),
                connected_channels: Vec::new(),
            }),
            output_receiver,
        )
    }

    pub(crate) fn push_chunk(&self, chunk: LogicChunk) -> Result<(), BridgeError> {
        if self.scheduler.is_none() {
            return Err(BridgeError::RawInputUnavailable);
        }
        map_input_send(self.input_sender.try_send(InputMessage::Chunk(chunk)))
    }

    pub(crate) fn finish(&self) -> Result<(), BridgeError> {
        if self.scheduler.is_none() {
            return Err(BridgeError::RawInputUnavailable);
        }
        map_input_send(self.input_sender.try_send(InputMessage::Finish))
    }

    pub(crate) fn cancel(&self) {
        let _ = self.cancel_sender.try_send(());
    }

    pub(crate) fn wait(&self, request: WaitRequest) -> Result<SchedulerStatus, BridgeError> {
        let scheduler = self
            .scheduler
            .as_ref()
            .ok_or(BridgeError::WaitUnavailable)?;
        let mut status = scheduler.lock().unwrap().begin_wait(request)?;
        loop {
            if status != SchedulerStatus::Waiting {
                return Ok(status);
            }
            select_biased! {
                recv(self.cancel_receiver) -> _ => {
                    return Ok(scheduler.lock().unwrap().cancel());
                }
                recv(self.input_receiver) -> message => {
                    status = match message.map_err(|_| BridgeError::InputQueueClosed)? {
                        InputMessage::Chunk(chunk) => scheduler.lock().unwrap().push_chunk(chunk)?,
                        InputMessage::Finish => scheduler.lock().unwrap().finish()?,
                    };
                }
            }
        }
    }

    pub(crate) fn register(&self, registration: OutputRegistration) -> usize {
        let mut registrations = self.registrations.lock().unwrap();
        if let Some((id, _)) = registrations
            .iter()
            .enumerate()
            .find(|(_, existing)| **existing == registration)
        {
            return id;
        }
        let id = registrations.len();
        registrations.push(registration);
        id
    }

    pub(crate) fn put(&self, output: DecoderOutput) -> Result<(), BridgeError> {
        if output.end_sample < output.start_sample {
            return Err(BridgeError::ReversedOutputRange);
        }
        if output.output_id >= self.registrations.lock().unwrap().len() {
            return Err(BridgeError::UnknownOutput(output.output_id));
        }
        select_biased! {
            recv(self.cancel_receiver) -> _ => Err(BridgeError::Cancelled),
            send(self.output_sender, output) -> result => {
                result.map_err(|_| BridgeError::OutputQueueClosed)
            }
        }
    }

    pub(crate) fn has_channel(&self, channel: usize) -> bool {
        self.connected_channels
            .get(channel)
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn registrations(&self) -> Vec<OutputRegistration> {
        self.registrations.lock().unwrap().clone()
    }
}

fn map_input_send(result: Result<(), TrySendError<InputMessage>>) -> Result<(), BridgeError> {
    result.map_err(|error| match error {
        TrySendError::Full(_) => BridgeError::InputQueueFull,
        TrySendError::Disconnected(_) => BridgeError::InputQueueClosed,
    })
}

pub(crate) fn matched_parts(result: WaitMatch) -> (u64, Vec<u8>, Option<Vec<bool>>) {
    (result.sample, result.pins, result.matched)
}

#[cfg(test)]
mod bridge_tests {
    use pyo3::Python;

    use super::super::scheduler::InitialPin;
    use super::*;

    #[test]
    fn full_output_queue_applies_backpressure_without_truncating_decoder_output() {
        Python::initialize();
        let (bridge, outputs) = DecoderBridge::new(vec![Some(InitialPin::Low)], 1).unwrap();
        let output_id = bridge.register(OutputRegistration {
            output_type: 0,
            protocol_id: None,
            metadata: None,
        });
        let output = move |sample| DecoderOutput {
            start_sample: sample,
            end_sample: sample,
            output_id,
            data: Python::attach(|py| py.None()),
        };

        bridge.put(output(1)).unwrap();
        let blocked_bridge = Arc::clone(&bridge);
        let blocked = std::thread::spawn(move || blocked_bridge.put(output(2)));

        assert_eq!(outputs.recv().unwrap().start_sample, 1);
        blocked.join().unwrap().unwrap();
        assert_eq!(outputs.recv().unwrap().start_sample, 2);
    }

    #[test]
    fn cancellation_interrupts_output_backpressure() {
        Python::initialize();
        let (bridge, _outputs) = DecoderBridge::new(vec![Some(InitialPin::Low)], 1).unwrap();
        let output_id = bridge.register(OutputRegistration {
            output_type: 0,
            protocol_id: None,
            metadata: None,
        });
        let output = move |sample| DecoderOutput {
            start_sample: sample,
            end_sample: sample,
            output_id,
            data: Python::attach(|py| py.None()),
        };

        bridge.put(output(1)).unwrap();
        let blocked_bridge = Arc::clone(&bridge);
        let blocked = std::thread::spawn(move || blocked_bridge.put(output(2)));
        bridge.cancel();

        assert!(matches!(
            blocked.join().unwrap(),
            Err(BridgeError::Cancelled)
        ));
    }
}
