use std::sync::Arc;

use signal_processing::{OutputPort, Sample, SampleBlock, Sender, WorkError, WorkOutcome};

const SAMPLES_PER_STEP: usize = 64 * 1024;

enum Stream {
    Edge(EdgeStream),
    Block(BlockStream),
}

struct EdgeStream {
    channel: usize,
    sender: Sender<Sample>,
    position: Option<usize>,
    value: bool,
}

struct BlockStream {
    channel: usize,
    sender: Sender<SampleBlock>,
    position: usize,
}

pub(crate) struct CooperativeSigrokReader {
    samples: Arc<[u8]>,
    unitsize: usize,
    total_samples: usize,
    timestamp_step: u64,
    streams: Vec<Stream>,
    next_stream: usize,
}

impl CooperativeSigrokReader {
    pub(crate) fn new(
        samples: Arc<[u8]>,
        unitsize: usize,
        total_samples: usize,
        samplerate_hz: f64,
        outputs: &[OutputPort],
    ) -> Self {
        let mut streams = Vec::new();
        for (channel, output) in outputs.iter().enumerate() {
            for sender in output.split_senders::<Sample>().unwrap_or_default() {
                streams.push(Stream::Edge(EdgeStream {
                    channel,
                    sender,
                    position: None,
                    value: false,
                }));
            }
            for sender in output.split_senders::<SampleBlock>().unwrap_or_default() {
                streams.push(Stream::Block(BlockStream {
                    channel,
                    sender,
                    position: 0,
                }));
            }
        }
        Self {
            samples,
            unitsize,
            total_samples,
            timestamp_step: (1_000_000_000.0 / samplerate_hz) as u64,
            streams,
            next_stream: 0,
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
            Stream::Edge(mut stream) => match self.step_edge(&mut stream) {
                Some(outcome) => (Some(Stream::Edge(stream)), outcome),
                None => (None, WorkOutcome::progressed(0)),
            },
            Stream::Block(mut stream) => match self.step_block(&mut stream) {
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

    fn step_edge(&self, stream: &mut EdgeStream) -> Option<WorkOutcome> {
        if self.total_samples == 0 {
            return None;
        }
        if stream.position.is_none() {
            stream.value = self.value_at(stream.channel, 0);
            stream.position = Some(0);
            return stream
                .sender
                .send(Sample::new(stream.value, 0))
                .is_ok()
                .then(|| WorkOutcome::progressed(1));
        }

        let position = stream.position.expect("initialized above");
        if position + 1 >= self.total_samples {
            return None;
        }
        let end = position
            .saturating_add(1 + SAMPLES_PER_STEP)
            .min(self.total_samples);
        for sample in position + 1..end {
            let value = self.value_at(stream.channel, sample);
            if value != stream.value {
                stream.value = value;
                stream.position = Some(sample);
                return stream
                    .sender
                    .send(Sample::new(
                        value,
                        (sample as u64).saturating_mul(self.timestamp_step),
                    ))
                    .is_ok()
                    .then(|| WorkOutcome::progressed(1));
            }
        }
        stream.position = Some(end - 1);
        Some(WorkOutcome::progressed(0))
    }

    fn step_block(&self, stream: &mut BlockStream) -> Option<WorkOutcome> {
        if stream.position >= self.total_samples {
            return None;
        }
        let count = SAMPLES_PER_STEP.min(self.total_samples - stream.position);
        let mut packed = vec![0_u8; count.div_ceil(8)];
        for offset in 0..count {
            if self.value_at(stream.channel, stream.position + offset) {
                packed[offset / 8] |= 1 << (offset % 8);
            }
        }
        let position = stream.position;
        stream.position += count;
        stream
            .sender
            .send(SampleBlock::new(
                packed,
                position as u64,
                count,
                self.timestamp_step,
            ))
            .is_ok()
            .then(|| WorkOutcome::progressed(1))
    }

    fn value_at(&self, channel: usize, sample: usize) -> bool {
        self.samples[sample * self.unitsize + channel / 8] & (1 << (channel % 8)) != 0
    }
}
