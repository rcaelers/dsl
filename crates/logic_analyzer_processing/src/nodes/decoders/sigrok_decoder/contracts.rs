//! Portable execution contract for one configured Sigrok decoder.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use signal_processing::{NodeCancellation, ProtocolPacket, ProtocolValue};

/// Initial logic level supplied to the Sigrok execution scheduler for a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialPin {
    /// Treat the channel as low before its first sample.
    Low,
    /// Treat the channel as high before its first sample.
    High,
    /// Infer the initial level from the first sample received for the channel.
    SameAsFirstSample,
}

/// A contiguous, bit-packed block of sampled logic delivered to an execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicChunk {
    /// Absolute index of the chunk's first sample.
    pub start_sample: u64,
    /// Number of valid samples in each populated channel buffer.
    pub sample_count: usize,
    /// One least-significant-bit-first packed buffer per configured channel; `None` is unconnected.
    pub channels: Vec<Option<Arc<[u8]>>>,
}

impl LogicChunk {
    /// Creates a contiguous block of packed channel samples.
    ///
    /// # Parameters
    /// - `start_sample`: Absolute index of the first sample in the block.
    /// - `sample_count`: Number of valid samples represented in each populated channel.
    /// - `channels`: Per-channel, least-significant-bit-first sample buffers; `None` denotes an
    ///   unconnected decoder channel.
    pub fn new(start_sample: u64, sample_count: usize, channels: Vec<Option<Arc<[u8]>>>) -> Self {
        Self {
            start_sample,
            sample_count,
            channels,
        }
    }

    /// Returns the number of samples in this chunk.
    pub const fn sample_count(&self) -> usize {
        self.sample_count
    }

    /// Returns the exclusive sample index after this chunk, or `None` on overflow.
    pub fn end_sample(&self) -> Option<u64> {
        self.start_sample.checked_add(self.sample_count as u64)
    }

    /// Returns one channel's logic level at its chunk-relative sample position.
    ///
    /// # Parameters
    /// - `channel`: Index into [`Self::channels`].
    /// - `sample`: Zero-based offset within this chunk.
    ///
    /// Returns `None` when the channel is unconnected. Callers must supply valid
    /// channel and sample indices.
    pub fn pin(&self, channel: usize, sample: usize) -> Option<bool> {
        self.channels[channel].as_ref().map(|data| {
            let byte = data[sample / 8];
            (byte >> (sample % 8)) & 1 != 0
        })
    }
}

/// Runtime value representation accepted by a Sigrok metadata output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataType {
    /// A signed integer metadata value.
    Integer,
    /// A floating-point metadata value.
    Float,
}

/// Metadata schema registered by a Sigrok decoder output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataRegistration {
    /// Value representation emitted under this registration.
    pub value_type: MetadataType,
    /// Upstream metadata key.
    pub name: String,
    /// Human-readable meaning of the key.
    pub description: String,
}

/// One output channel registered by a spawned Sigrok decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputRegistration {
    /// Upstream output type identifier.
    pub output_type: i32,
    /// Optional protocol identifier carried by protocol-packet output.
    pub protocol_id: Option<String>,
    /// Optional metadata schema for metadata output.
    pub metadata: Option<MetadataRegistration>,
}

/// Scalar option value declared by a discovered Sigrok decoder.
#[derive(Clone, Debug, PartialEq)]
pub enum SigrokScalarValue {
    /// Boolean value.
    Bool(bool),
    /// Signed integer value.
    Integer(i64),
    /// Floating-point value.
    Float(f64),
    /// UTF-8 string value.
    String(String),
}

/// Human-facing description of one required or optional decoder channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokDecoderChannelDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// One configurable option advertised by a decoder package.
#[derive(Clone, Debug, PartialEq)]
pub struct SigrokDecoderOptionDescriptor {
    pub id: String,
    pub description: String,
    pub default: SigrokScalarValue,
    pub values: Vec<SigrokScalarValue>,
}

/// An annotation category emitted by a decoder package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokAnnotationClassDescriptor {
    pub id: String,
    pub description: String,
}

/// A displayed annotation row and the annotation classes it accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokAnnotationRowDescriptor {
    pub id: String,
    pub description: String,
    pub classes: Vec<usize>,
}

/// Kind of data a Sigrok decoder emits through a registered output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigrokOutputKind {
    /// Time-ranged visual annotation.
    Annotation,
    /// Arbitrary binary data.
    Binary,
    /// Generated digital logic samples.
    GeneratedLogic,
    /// Typed metadata value.
    Metadata,
    /// Structured protocol packet.
    ProtocolPacket,
}

/// Complete portable description discovered from one Sigrok decoder package.
#[derive(Clone, Debug, PartialEq)]
pub struct SigrokDecoderDescriptor {
    pub api_version: i64,
    pub id: String,
    pub name: String,
    pub long_name: String,
    pub description: String,
    pub license: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub tags: Vec<String>,
    pub channels: Vec<SigrokDecoderChannelDescriptor>,
    pub optional_channels: Vec<SigrokDecoderChannelDescriptor>,
    pub options: Vec<SigrokDecoderOptionDescriptor>,
    pub annotations: Vec<SigrokAnnotationClassDescriptor>,
    pub annotation_rows: Vec<SigrokAnnotationRowDescriptor>,
    pub binary: Vec<SigrokAnnotationClassDescriptor>,
    pub logic_output_channels: Vec<SigrokDecoderChannelDescriptor>,
    pub registered_outputs: Vec<SigrokOutputKind>,
    pub package_fingerprint: String,
}

/// One externally discovered Sigrok decoder package.
#[derive(Clone, Debug, PartialEq)]
pub struct SigrokCatalogEntry {
    pub decoder_root: PathBuf,
    pub descriptor: SigrokDecoderDescriptor,
}

/// The reason a decoder-directory scan could not include a package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigrokCatalogDiagnosticKind {
    /// A configured search path does not exist.
    MissingSearchPath,
    /// A configured search path could not be enumerated.
    UnreadableSearchPath,
    /// A package could not be parsed or validated as a decoder.
    InvalidDecoder,
    /// A package duplicated an already discovered decoder identifier.
    DuplicateDecoder,
}

/// A non-fatal diagnostic produced while scanning external decoder packages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokCatalogDiagnostic {
    pub kind: SigrokCatalogDiagnosticKind,
    pub path: PathBuf,
    pub decoder_id: Option<String>,
    pub message: String,
}

/// The portable result of an external Sigrok decoder-package scan.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SigrokCatalogSnapshot {
    pub entries: Vec<SigrokCatalogEntry>,
    pub diagnostics: Vec<SigrokCatalogDiagnostic>,
}

/// A scalar option value transferred to a spawned decoder execution.
#[derive(Clone, Debug)]
pub enum SigrokExecutionOptionValue {
    /// Boolean option value.
    Bool(bool),
    /// Signed integer option value.
    Integer(i64),
    /// Floating-point option value.
    Float(f64),
    /// UTF-8 string option value.
    String(String),
}

/// Input transport selected for a spawned decoder execution.
#[derive(Clone, Debug)]
pub enum SigrokExecutionInput {
    /// Sampled logic channels and their initial-level policies.
    Logic(Vec<Option<InitialPin>>),
    /// Protocol stream identifiers accepted as input by the decoder.
    Protocol(Vec<String>),
}

/// Immutable inputs used to start one Sigrok decoder execution.
#[derive(Clone, Debug)]
pub struct SigrokExecutionConfig {
    pub decoder_root: PathBuf,
    pub decoder_id: String,
    pub sample_rate: u64,
    pub input: SigrokExecutionInput,
    pub options: BTreeMap<String, SigrokExecutionOptionValue>,
    pub queue_capacity: usize,
}

/// A time-ranged value emitted by a Sigrok execution.
#[derive(Clone, Debug, PartialEq)]
pub struct SigrokExecutionOutput {
    pub start_sample: u64,
    pub end_sample: u64,
    pub output_id: usize,
    pub data: ProtocolValue,
}

/// Running, asynchronous Sigrok decoder execution.
///
/// Callers submit input in capture order, request finalization once, drain outputs,
/// then call [`Self::join`] to release the worker. Implementations may block only
/// according to the supplied [`Duration`] in [`Self::receive_output`].
pub trait SigrokExecution: Send {
    /// Submits the next contiguous logic chunk to the decoder.
    ///
    /// # Parameters
    /// - `chunk`: Packed samples in capture order. Its range must follow previously submitted
    ///   chunks according to the execution's input contract.
    fn push_chunk(&self, chunk: LogicChunk) -> Result<(), String>;

    /// Submits the next structured packet for a protocol-input decoder.
    ///
    /// # Parameters
    /// - `packet`: Packet received from the graph input stream, in stream order.
    fn push_protocol_packet(&self, packet: ProtocolPacket) -> Result<(), String>;

    /// Signals end-of-input and asks the decoder to flush pending output.
    ///
    /// This may be called once after all input was submitted.
    fn finish(&self) -> Result<(), String>;

    /// Returns the cancellation handle observed by the underlying execution.
    fn cancellation(&self) -> Arc<dyn NodeCancellation>;

    /// Returns one immediately available output, without waiting.
    ///
    /// `Ok(None)` means no output is currently queued; it does not imply completion.
    fn try_output(&self) -> Result<Option<SigrokExecutionOutput>, String>;

    /// Returns output registrations declared by the initialized decoder.
    fn registrations(&self) -> Vec<OutputRegistration>;

    /// Returns whether the execution has completed and will not produce further output.
    fn is_finished(&self) -> bool;

    /// Waits for one output, completion, or the timeout.
    ///
    /// # Parameters
    /// - `timeout`: Maximum time to wait for a queued result.
    ///
    /// `Ok(None)` means the decoder completed or no output arrived before the timeout.
    fn receive_output(&self, timeout: Duration) -> Result<Option<SigrokExecutionOutput>, String>;

    /// Waits for the worker to stop and releases its resources.
    ///
    /// Call after [`Self::finish`] and output draining, or when propagating a terminal error.
    fn join(&mut self) -> Result<(), String>;
}

/// Factory for platform-specific Sigrok decoder execution workers.
pub trait SigrokExecutionFactory: Send + Sync {
    /// Starts an execution configured for one discovered decoder package.
    ///
    /// # Parameters
    /// - `config`: Package location, decoder identity, input transport, options, and queue limit.
    fn spawn(&self, config: SigrokExecutionConfig) -> Result<Box<dyn SigrokExecution>, String>;
}
