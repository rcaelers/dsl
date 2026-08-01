//! Portable execution contract for one configured Sigrok decoder.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use signal_processing::{NodeCancellation, ProtocolPacket, ProtocolValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitialPin {
    Low,
    High,
    SameAsFirstSample,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicChunk {
    pub start_sample: u64,
    pub sample_count: usize,
    pub channels: Vec<Option<Arc<[u8]>>>,
}

impl LogicChunk {
    pub fn new(start_sample: u64, sample_count: usize, channels: Vec<Option<Arc<[u8]>>>) -> Self {
        Self {
            start_sample,
            sample_count,
            channels,
        }
    }

    pub const fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn end_sample(&self) -> Option<u64> {
        self.start_sample.checked_add(self.sample_count as u64)
    }

    pub fn pin(&self, channel: usize, sample: usize) -> Option<bool> {
        self.channels[channel].as_ref().map(|data| {
            let byte = data[sample / 8];
            (byte >> (sample % 8)) & 1 != 0
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataType {
    Integer,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataRegistration {
    pub value_type: MetadataType,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputRegistration {
    pub output_type: i32,
    pub protocol_id: Option<String>,
    pub metadata: Option<MetadataRegistration>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SigrokScalarValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokDecoderChannelDescriptor {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SigrokDecoderOptionDescriptor {
    pub id: String,
    pub description: String,
    pub default: SigrokScalarValue,
    pub values: Vec<SigrokScalarValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokAnnotationClassDescriptor {
    pub id: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokAnnotationRowDescriptor {
    pub id: String,
    pub description: String,
    pub classes: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigrokOutputKind {
    Annotation,
    Binary,
    GeneratedLogic,
    Metadata,
    ProtocolPacket,
}

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
    MissingSearchPath,
    UnreadableSearchPath,
    InvalidDecoder,
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

#[derive(Clone, Debug)]
pub enum SigrokExecutionOptionValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

#[derive(Clone, Debug)]
pub enum SigrokExecutionInput {
    Logic(Vec<Option<InitialPin>>),
    Protocol(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct SigrokExecutionConfig {
    pub decoder_root: PathBuf,
    pub decoder_id: String,
    pub sample_rate: u64,
    pub input: SigrokExecutionInput,
    pub options: BTreeMap<String, SigrokExecutionOptionValue>,
    pub queue_capacity: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SigrokExecutionOutput {
    pub start_sample: u64,
    pub end_sample: u64,
    pub output_id: usize,
    pub data: ProtocolValue,
}

pub trait SigrokExecution: Send {
    fn push_chunk(&self, chunk: LogicChunk) -> Result<(), String>;

    fn push_protocol_packet(&self, packet: ProtocolPacket) -> Result<(), String>;

    fn finish(&self) -> Result<(), String>;

    fn cancellation(&self) -> Arc<dyn NodeCancellation>;

    fn try_output(&self) -> Result<Option<SigrokExecutionOutput>, String>;

    fn registrations(&self) -> Vec<OutputRegistration>;

    fn is_finished(&self) -> bool;

    fn receive_output(&self, timeout: Duration) -> Result<Option<SigrokExecutionOutput>, String>;

    fn join(&mut self) -> Result<(), String>;
}

pub trait SigrokExecutionFactory: Send + Sync {
    fn spawn(&self, config: SigrokExecutionConfig) -> Result<Box<dyn SigrokExecution>, String>;
}
