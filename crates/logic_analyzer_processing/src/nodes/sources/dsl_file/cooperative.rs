use std::sync::{Arc, Mutex};

use signal_capture::{Sample, SampleBlock};
use signal_runtime::{OutputPort, Sender, WorkError, WorkOutcome};

use crate::support::dsl_file::DslChunkedCaptureReader;

enum Stream {
    Edge(EdgeStream),
    Block(BlockStream),
}

struct EdgeStream {
    channel: usize,
    sender: Sender<Sample>,
    position: Option<u64>,
}

struct BlockStream {
    channel: usize,
    sender: Sender<SampleBlock>,
    block: u64,
}

pub(crate) struct CooperativeDslReader {
    sampler: Arc<Mutex<DslChunkedCaptureReader>>,
    streams: Vec<Stream>,
    next_stream: usize,
    total_samples: u64,
    total_blocks: u64,
    samples_per_block: u64,
    timestamp_step: u64,
}

impl CooperativeDslReader {
    pub(crate) fn new(
        sampler: Arc<Mutex<DslChunkedCaptureReader>>,
        outputs: &[OutputPort],
        total_samples: u64,
        total_blocks: u64,
        samples_per_block: u64,
        samplerate_hz: f64,
    ) -> Self {
        let mut streams = Vec::new();
        for (channel, output) in outputs.iter().enumerate() {
            for sender in output.split_senders::<Sample>().unwrap_or_default() {
                streams.push(Stream::Edge(EdgeStream {
                    channel,
                    sender,
                    position: None,
                }));
            }
            for sender in output.split_senders::<SampleBlock>().unwrap_or_default() {
                streams.push(Stream::Block(BlockStream {
                    channel,
                    sender,
                    block: 0,
                }));
            }
        }
        Self {
            sampler,
            streams,
            next_stream: 0,
            total_samples,
            total_blocks,
            samples_per_block,
            timestamp_step: (1_000_000_000.0 / samplerate_hz) as u64,
        }
    }

    pub(crate) fn step(&mut self) -> Result<WorkOutcome, WorkError> {
        if self.streams.is_empty() {
            return Err(WorkError::Shutdown);
        }

        if self.next_stream >= self.streams.len() {
            self.next_stream = 0;
        }
        let stream = self.streams.remove(self.next_stream);
        let (stream, outcome) = match stream {
            Stream::Edge(mut stream) => match self.step_edge(&mut stream)? {
                Some(outcome) => (Some(Stream::Edge(stream)), outcome),
                None => (None, WorkOutcome::progressed(0)),
            },
            Stream::Block(mut stream) => match self.step_block(&mut stream)? {
                Some(outcome) => (Some(Stream::Block(stream)), outcome),
                None => (None, WorkOutcome::progressed(0)),
            },
        };
        if let Some(stream) = stream {
            self.streams.insert(self.next_stream, stream);
            self.next_stream = (self.next_stream + 1) % self.streams.len();
        } else if !self.streams.is_empty() {
            self.next_stream %= self.streams.len();
        }
        Ok(outcome)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.streams.is_empty()
    }

    fn step_edge(&self, stream: &mut EdgeStream) -> Result<Option<WorkOutcome>, WorkError> {
        if self.total_samples == 0 {
            return Ok(None);
        }
        let mut sampler = self.sampler.lock().unwrap();
        let transition = if let Some(position) = stream.position {
            sampler
                .next_transition(stream.channel, position, self.total_samples)
                .map_err(|error| WorkError::NodeError(error.to_string()))?
        } else {
            let value = sampler
                .value_at(stream.channel, 0)
                .map_err(|error| WorkError::NodeError(error.to_string()))?;
            drop(sampler);
            if stream.sender.send(Sample::new(value, 0)).is_err() {
                return Ok(None);
            }
            stream.position = Some(0);
            return Ok(Some(WorkOutcome::progressed(1)));
        };
        drop(sampler);

        let Some(transition) = transition else {
            return Ok(None);
        };
        if stream
            .sender
            .send(Sample::new(
                transition.value,
                transition.sample.saturating_mul(self.timestamp_step),
            ))
            .is_err()
        {
            return Ok(None);
        }
        stream.position = Some(transition.sample);
        Ok(Some(WorkOutcome::progressed(1)))
    }

    fn step_block(&self, stream: &mut BlockStream) -> Result<Option<WorkOutcome>, WorkError> {
        if stream.block >= self.total_blocks {
            return Ok(None);
        }
        let block_start = stream.block.saturating_mul(self.samples_per_block);
        if block_start >= self.total_samples {
            return Ok(None);
        }
        let data = self
            .sampler
            .lock()
            .unwrap()
            .packed_block(stream.channel, stream.block)
            .map_err(|error| WorkError::NodeError(error.to_string()))?;
        let samples = (data.len() as u64)
            .saturating_mul(8)
            .min(self.total_samples - block_start) as usize;
        if stream
            .sender
            .send(SampleBlock::new(
                data,
                block_start,
                samples,
                self.timestamp_step,
            ))
            .is_err()
        {
            return Ok(None);
        }
        stream.block += 1;
        Ok(Some(WorkOutcome::progressed(1)))
    }
}
