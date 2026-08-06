//! Common logic-analyzer capture interface and processing source-node adapter.
//!
//! Drivers return packed, interleaved data exactly as acquired.  The adapter
//! exposes the existing `dN`/`bN` node ports, so a hardware capture can be
//! connected to the same decoders as a `.dsl` replay.  A future libsigrok
//! adapter only needs to implement [`LogicAnalyzer`].

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use thiserror::Error;

use platform_runtime::{WorkExecutor, WorkTask};
use signal_capture::{Sample, SampleBlock};
use signal_runtime::{
    InputPort, OutputPort, PortDirection, PortPayload, PortSchema, ProcessNode, Sender, WorkError,
    WorkResult,
};

use super::trigger::LogicTrigger;

/// Static capabilities exposed by a logic-analyzer driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicAnalyzerInfo {
    /// Stable driver identifier used for diagnostics and source naming.
    pub driver: String,
    /// Human-readable device model.
    pub model: String,
    /// Number of physical digital inputs exposed by the device.
    pub channels: u8,
    /// Sample rates the device supports in hertz.
    pub sample_rates_hz: Vec<u64>,
}

/// Capture mode supported by most logic analyzers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Capture continues until the caller stops it.
    Streaming,
    /// Capture stops after the configured sample limit.
    Finite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogicEncodingRequest {
    /// Request uncompressed interleaved digital samples.
    #[default]
    Raw,
    /// Request driver-supported run-length compression.
    RunLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEdge {
    /// Sample on an external clock's rising edge.
    Rising,
    /// Sample on an external clock's falling edge.
    Falling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClockSource {
    /// Use the analyzer's internal sampling clock.
    #[default]
    Internal,
    External {
        /// Edge of the external clock used for sampling.
        edge: ClockEdge,
    },
}

/// Vendor-neutral capture request. Inputs are physical-channel bits; the node
/// exposes the enabled inputs in increasing physical-channel order.
#[derive(Debug, Clone)]
pub struct LogicCaptureConfig {
    /// Whether capture is finite or continues until stopped.
    pub mode: CaptureMode,
    /// Requested sampling rate in hertz.
    pub sample_rate_hz: u64,
    /// Bitmask of enabled physical inputs.
    pub input_mask: u64,
    /// Maximum samples for finite captures.
    pub sample_limit: u64,
    /// Trigger position within a finite capture, as a percentage from 0 to 100.
    pub trigger_percent: u8,
    /// A common logic-analyzer control. Drivers without a threshold DAC reject it.
    pub threshold_volts: Option<f32>,
    /// Portable multi-stage trigger program.
    pub trigger: LogicTrigger,
    /// Requested wire encoding.
    pub encoding: LogicEncodingRequest,
    /// Sampling-clock source.
    pub clock: ClockSource,
    /// Whether the driver should apply its digital input filter.
    pub input_filter: bool,
}

impl LogicCaptureConfig {
    /// Creates a finite capture request with conservative defaults.
    ///
    /// # Parameters
    /// - `sample_rate_hz`: Requested sampling rate in hertz.
    /// - `input_mask`: Bitmask of enabled physical inputs.
    /// - `sample_limit`: Number of samples to retain.
    pub fn finite(sample_rate_hz: u64, input_mask: u64, sample_limit: u64) -> Self {
        Self {
            mode: CaptureMode::Finite,
            sample_rate_hz,
            input_mask,
            sample_limit,
            trigger_percent: 50,
            threshold_volts: None,
            trigger: LogicTrigger::default(),
            encoding: LogicEncodingRequest::Raw,
            clock: ClockSource::Internal,
            input_filter: false,
        }
    }
}

/// Data encoding returned by a driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicEncoding {
    /// Samples are LSB-first, with enabled inputs in increasing input-number order.
    InterleavedLsbFirst,
    /// Device-specific encoded data. It is delivered only through the `raw` port.
    Opaque,
}

/// A lossless piece of a raw logic-data stream.
///
/// `bit_offset` and `bit_len` identify the valid span in `data`; they permit a
/// driver to preserve transfer boundaries that occur in the middle of a sample.
#[derive(Clone, Debug)]
pub struct LogicChunk {
    /// Raw transfer bytes containing the valid bit span.
    pub data: Arc<[u8]>,
    /// Index of the first valid bit within `data`.
    pub bit_offset: u8,
    /// Number of valid bits beginning at [`Self::bit_offset`].
    pub bit_len: usize,
    /// Number of interleaved enabled input channels.
    pub channel_count: u8,
    /// Absolute bit position of the first bit in the capture stream.
    pub start_bit: u64,
    /// Encoding used for this transfer.
    pub encoding: LogicEncoding,
}

impl LogicChunk {
    #[cfg(test)]
    pub(crate) fn interleaved(
        data: impl Into<Arc<[u8]>>,
        channel_count: u8,
        start_bit: u64,
    ) -> Self {
        let data = data.into();
        Self {
            bit_len: data.len() * 8,
            data,
            bit_offset: 0,
            channel_count,
            start_bit,
            encoding: LogicEncoding::InterleavedLsbFirst,
        }
    }

    #[inline]
    /// Returns one bit from the chunk's valid span.
    ///
    /// # Parameters
    ///
    /// - `relative_bit`: Zero-based bit position within the valid span.
    pub fn bit(&self, relative_bit: usize) -> bool {
        debug_assert!(relative_bit < self.bit_len);
        let absolute = usize::from(self.bit_offset) + relative_bit;
        (self.data[absolute / 8] >> (absolute % 8)) & 1 != 0
    }
}

/// A driver-independent capture error.
#[derive(Debug, Error)]
pub enum LogicAnalyzerError {
    /// The requested mode, channels, or trigger configuration is invalid.
    #[error("invalid capture settings: {0}")]
    InvalidSettings(String),
    /// Communication with the device failed.
    #[error("transport error: {0}")]
    Transport(String),
    /// The driver or device returned an invalid protocol response.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Capture data violates its declared encoding or bounds.
    #[error("capture integrity error: {0}")]
    Integrity(String),
    /// A device operation did not complete before its deadline.
    #[error("operation timed out: {0}")]
    Timeout(String),
    /// An operation requiring an active capture was requested while idle.
    #[error("capture is not active")]
    NotCapturing,
}

/// Result alias for device-driver operations.
pub type LogicAnalyzerResult<T> = std::result::Result<T, LogicAnalyzerError>;

/// The minimal interface required by the runtime and by prospective C-driver
/// bridges. Implementations must serialize their device control path.
pub trait LogicAnalyzer: Send + 'static {
    /// Returns static device capabilities.
    fn info(&self) -> &LogicAnalyzerInfo;
    /// Validates and applies capture settings while the device is idle.
    ///
    /// # Parameters
    /// - `config`: Complete capture request to apply.
    fn configure_capture(&mut self, config: &LogicCaptureConfig) -> LogicAnalyzerResult<()>;
    /// The rate of the active capture. Valid after `start_capture` succeeds.
    fn sample_rate_hz(&self) -> u64;
    /// Starts acquisition using the most recently configured settings.
    fn start_capture(&mut self) -> LogicAnalyzerResult<()>;
    /// Returns the next raw transfer, or `None` when a finite capture ends.
    fn next_chunk(&mut self) -> LogicAnalyzerResult<Option<LogicChunk>>;
    /// Stops an active capture and releases device-side acquisition state.
    fn stop_capture(&mut self) -> LogicAnalyzerResult<()>;
}

/// Turns any [`LogicAnalyzer`] into a graph source.
///
/// Output names retain the file-source convention: `d0..dN` are transition
/// streams, `b0..bN` are aligned packed blocks, and `raw` exposes lossless
/// driver chunks. Logical input N is the Nth enabled hardware input.
pub struct LogicAnalyzerSource<A: LogicAnalyzer> {
    name: String,
    analyzer: Option<A>,
    channels: u8,
    config: LogicCaptureConfig,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
    task: Option<Box<dyn WorkTask>>,
    work_executor: Arc<dyn WorkExecutor>,
    started: bool,
}

impl<A: LogicAnalyzer> LogicAnalyzerSource<A> {
    /// Creates a graph source from one configured device driver.
    ///
    /// # Parameters
    ///
    /// - `analyzer`: Driver that owns the physical device.
    /// - `config`: Validated portable capture request.
    pub fn new(analyzer: A, config: LogicCaptureConfig) -> LogicAnalyzerResult<Self> {
        signal_runtime::register_type::<LogicChunk>();
        let channels = config.input_mask.count_ones() as u8;
        if channels == 0
            || channels > analyzer.info().channels
            || config.sample_limit == 0
            || config.trigger_percent > 100
        {
            return Err(LogicAnalyzerError::InvalidSettings(
                "invalid channel mask, sample limit, or trigger percentage".into(),
            ));
        }
        Ok(Self {
            name: format!("{}_source", analyzer.info().driver),
            analyzer: Some(analyzer),
            channels,
            config,
            shutdown: Arc::new(AtomicBool::new(false)),
            completed: Arc::new(AtomicUsize::new(0)),
            task: None,
            work_executor: Arc::new(platform_runtime::InlineWorkExecutor),
            started: false,
        })
    }

    /// Sets the graph-local name for this source.
    ///
    /// # Parameters
    ///
    /// - `name`: Name shown in graph and runtime diagnostics.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Supplies the host-selected executor for the long-lived capture task.
    ///
    /// # Parameters
    ///
    /// - `work_executor`: Host capability that owns the capture task.
    pub fn with_work_executor(mut self, work_executor: Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = work_executor;
        self
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        mut analyzer: A,
        config: LogicCaptureConfig,
        channels: usize,
        edge_senders: Vec<Option<Sender<Sample>>>,
        block_senders: Vec<Option<Sender<SampleBlock>>>,
        raw_sender: Option<Sender<LogicChunk>>,
        shutdown: Arc<AtomicBool>,
        completed: Arc<AtomicUsize>,
    ) {
        let result = (|| -> LogicAnalyzerResult<()> {
            analyzer.configure_capture(&config)?;
            analyzer.start_capture()?;
            let mut demux = Demux::new(channels, analyzer.sample_rate_hz())?;
            while !shutdown.load(Ordering::Relaxed) {
                let Some(chunk) = analyzer.next_chunk()? else {
                    break;
                };
                // Drivers may yield an empty chunk after a bounded transport
                // timeout. This lets streaming sources observe shutdown
                // without treating an idle bus as end of capture.
                if chunk.bit_len == 0 {
                    continue;
                }
                if let Some(sender) = &raw_sender
                    && sender.send(chunk.clone()).is_err()
                {
                    // Other ports may still be connected; do not stop capture.
                }
                if chunk.encoding == LogicEncoding::Opaque {
                    continue;
                }
                demux.push(&chunk, &edge_senders, &block_senders)?;
            }
            analyzer.stop_capture()?;
            demux.finish(&edge_senders);
            Ok(())
        })();

        if let Err(error) = result {
            tracing::error!(%error, "logic-analyzer source stopped with an error");
        }
        for sender in edge_senders.iter().flatten() {
            sender.close();
        }
        for sender in block_senders.iter().flatten() {
            sender.close();
        }
        if let Some(sender) = raw_sender {
            sender.close();
        }
        completed.fetch_add(1, Ordering::Relaxed);
    }
}

impl<A: LogicAnalyzer> ProcessNode for LogicAnalyzerSource<A> {
    fn name(&self) -> &str {
        &self.name
    }
    fn should_stop(&self) -> bool {
        self.started && self.completed.load(Ordering::Relaxed) != 0
    }
    fn is_self_threading(&self) -> bool {
        true
    }
    fn num_inputs(&self) -> usize {
        0
    }
    fn num_outputs(&self) -> usize {
        // One port per channel (`ch0..chN`, negotiates Sample vs
        // SampleBlock per connection) plus the raw chunk port.
        usize::from(self.channels) + 1
    }

    fn output_schema(&self) -> Vec<PortSchema> {
        let n = usize::from(self.channels);
        let mut schema = Vec::with_capacity(n + 1);
        for i in 0..n {
            schema.push(
                PortSchema::state::<Sample>(format!("ch{i}"), i, PortDirection::Output)
                    // Block is a near-zero-cost passthrough of the
                    // packed-bit chunk already captured; Edge costs a real
                    // bit-walk to derive RLE edges (see `Demux::push`) —
                    // prefer Block, but a consumer that only wants Edge
                    // still gets it.
                    .with_payloads(vec![
                        PortPayload::new::<SampleBlock>().with_default_buffer_capacity(2),
                        PortPayload::new::<Sample>().state(),
                    ]),
            );
        }
        schema.push(PortSchema::new::<LogicChunk>(
            "raw",
            n,
            PortDirection::Output,
        ));
        schema
    }

    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        if self.started {
            return Err(WorkError::NodeError(
                "work() called twice on logic-analyzer source".into(),
            ));
        }
        self.started = true;
        let analyzer = self
            .analyzer
            .take()
            .ok_or_else(|| WorkError::NodeError("missing analyzer".into()))?;
        let n = usize::from(self.channels);
        let config = self.config.clone();
        // Both queries run independently against the same channel port —
        // it can carry Sample and SampleBlock destinations simultaneously
        // (negotiated per connection, see `output_sample_kinds`).
        let edge_senders = (0..n)
            .map(|i| outputs.get(i).and_then(|p| p.clone_sender::<Sample>()))
            .collect();
        let block_senders = (0..n)
            .map(|i| outputs.get(i).and_then(|p| p.clone_sender::<SampleBlock>()))
            .collect();
        let raw_sender = outputs.get(n).and_then(|p| p.clone_sender::<LogicChunk>());
        let shutdown = Arc::clone(&self.shutdown);
        let completed = Arc::clone(&self.completed);
        self.task = Some(
            self.work_executor
                .submit_long_running(Box::new(move || {
                    Self::run(
                        analyzer,
                        config,
                        n,
                        edge_senders,
                        block_senders,
                        raw_sender,
                        shutdown,
                        completed,
                    );
                }))
                .map_err(|error| {
                    WorkError::NodeError(format!("cannot start logic-analyzer source: {error}"))
                })?,
        );
        Ok(0)
    }
}

impl<A: LogicAnalyzer> Drop for LogicAnalyzerSource<A> {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(task) = self.task.take() {
            task.wait();
        }
    }
}

struct Demux {
    channels: usize,
    sample_rate_hz: u64,
    pending: Vec<bool>,
    position: u64,
    next_bit: Option<u64>,
    values: Vec<Option<bool>>,
}

impl Demux {
    fn new(channels: usize, sample_rate_hz: u64) -> LogicAnalyzerResult<Self> {
        if sample_rate_hz == 0 {
            return Err(LogicAnalyzerError::Protocol(
                "driver reported a zero sample rate".into(),
            ));
        }
        Ok(Self {
            channels,
            sample_rate_hz,
            pending: Vec::with_capacity(channels),
            position: 0,
            next_bit: None,
            values: vec![None; channels],
        })
    }

    fn push(
        &mut self,
        chunk: &LogicChunk,
        edges: &[Option<Sender<Sample>>],
        blocks: &[Option<Sender<SampleBlock>>],
    ) -> LogicAnalyzerResult<()> {
        if chunk.bit_offset >= 8
            || usize::from(chunk.bit_offset)
                .checked_add(chunk.bit_len)
                .is_none_or(|end| end > chunk.data.len() * 8)
        {
            return Err(LogicAnalyzerError::Protocol(
                "logic chunk bit range is outside its backing data".into(),
            ));
        }
        if usize::from(chunk.channel_count) != self.channels {
            return Err(LogicAnalyzerError::Protocol(format!(
                "chunk has {} channels; source expects {}",
                chunk.channel_count, self.channels
            )));
        }
        if let Some(expected) = self.next_bit
            && chunk.start_bit != expected
        {
            return Err(LogicAnalyzerError::Protocol(format!(
                "non-contiguous chunk: starts at bit {}, expected {expected}",
                chunk.start_bit
            )));
        }
        self.next_bit = Some(
            chunk
                .start_bit
                .checked_add(chunk.bit_len as u64)
                .ok_or_else(|| {
                    LogicAnalyzerError::Protocol("chunk bit position overflow".into())
                })?,
        );
        let start = self.position;
        let mut packed = vec![Vec::<u8>::new(); self.channels];
        let mut samples = 0usize;
        for bit in 0..chunk.bit_len {
            self.pending.push(chunk.bit(bit));
            if self.pending.len() != self.channels {
                continue;
            }
            for channel in 0..self.channels {
                let value = self.pending[channel];
                if self.values[channel] != Some(value) {
                    if let Some(sender) = &edges[channel] {
                        let _ = sender.send(Sample::new(value, self.timestamp(self.position)));
                    }
                    self.values[channel] = Some(value);
                }
                if samples.is_multiple_of(8) {
                    packed[channel].push(0);
                }
                if value {
                    let byte = packed[channel].last_mut().unwrap();
                    *byte |= 1 << (samples % 8);
                }
            }
            self.pending.clear();
            self.position += 1;
            samples += 1;
        }
        if samples != 0 {
            for (channel, data) in packed.into_iter().enumerate() {
                if let Some(sender) = &blocks[channel] {
                    let _ = sender.send(SampleBlock::new(
                        data,
                        start,
                        samples,
                        1_000_000_000 / self.sample_rate_hz,
                    ));
                }
            }
        }
        Ok(())
    }

    fn finish(&self, edges: &[Option<Sender<Sample>>]) {
        for (channel, value) in self.values.iter().enumerate() {
            if let (Some(sender), Some(value)) = (&edges[channel], value) {
                let _ = sender.send(Sample::new(*value, self.timestamp(self.position)));
            }
        }
    }

    fn timestamp(&self, position: u64) -> u64 {
        position.saturating_mul(1_000_000_000) / self.sample_rate_hz
    }
}

impl fmt::Display for LogicChunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LogicChunk[bits={}, channels={}]",
            self.bit_len, self.channel_count
        )
    }
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::bounded;
    use signal_runtime::ChannelMessage;

    use super::*;

    #[test]
    fn demux_emits_aligned_owned_channel_blocks() {
        let channel0 = [false, true, false, true, true, false, true, false];
        let channel1 = [true, true, false, false, true, true, false, false];
        let mut interleaved = vec![0u8; 2];
        for sample in 0..8 {
            for (channel, values) in [channel0, channel1].iter().enumerate() {
                if values[sample] {
                    let bit = sample * 2 + channel;
                    interleaved[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        let chunk = LogicChunk::interleaved(interleaved, 2, 0);
        let (tx0, rx0) = bounded::<ChannelMessage<SampleBlock>>(1);
        let (tx1, rx1) = bounded::<ChannelMessage<SampleBlock>>(1);
        let blocks = vec![Some(Sender::new(vec![tx0])), Some(Sender::new(vec![tx1]))];
        let edges: Vec<Option<Sender<Sample>>> = vec![None, None];
        let mut demux = Demux::new(2, 50_000_000).unwrap();

        demux.push(&chunk, &edges, &blocks).unwrap();

        let block0 = match rx0.recv().unwrap() {
            ChannelMessage::Sample(block) => block,
            ChannelMessage::Batch(_) => panic!("unexpected batch"),
            ChannelMessage::EndOfStream => panic!("unexpected end of stream"),
        };
        let block1 = match rx1.recv().unwrap() {
            ChannelMessage::Sample(block) => block,
            ChannelMessage::Batch(_) => panic!("unexpected batch"),
            ChannelMessage::EndOfStream => panic!("unexpected end of stream"),
        };
        assert_eq!(&*block0.data, &[0b0101_1010]);
        assert_eq!(&*block1.data, &[0b0011_0011]);
        assert_eq!(block0.start_position, block1.start_position);
        assert_eq!(block0.num_samples, 8);
        assert_eq!(block1.num_samples, 8);
    }
}
