use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::preparation::CaptureIndexPreparationRequest;
use crate::{
    ByteRange, ByteRegion, Error, ImmutableByteRegion, Result, SourceIdentity, WorkExecutor,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureMetadata {
    /// Total number of probes/channels.
    pub total_probes: usize,
    /// Sample rate as a display string, e.g. "50 MHz".
    pub samplerate: String,
    /// Sample rate in Hz.
    pub samplerate_hz: f64,
    /// Sample period in seconds.
    pub sample_period: f64,
    /// Total number of samples currently available.
    ///
    /// For finite file captures this is final. For future live captures this can
    /// grow over time.
    pub total_samples: u64,
    /// Total number of packed data blocks currently available.
    pub total_blocks: u64,
    /// Samples per packed block.
    pub samples_per_block: u64,
    /// Probe names indexed by probe number.
    pub probe_names: Vec<String>,
    /// Raw sample at which an acquisition trigger matched, when one was observed.
    pub trigger_sample: Option<u64>,
}

impl CaptureMetadata {
    pub fn duration_us(&self) -> f64 {
        self.total_samples as f64 * 1_000_000.0 / self.samplerate_hz
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureTransition {
    pub sample: u64,
    pub value: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureWaveformSegment {
    Level {
        start_sample: u64,
        end_sample: u64,
        value: bool,
    },
    Edge {
        sample: u64,
        before: bool,
        after: bool,
    },
    Activity {
        start_sample: u64,
        end_sample: u64,
        first: bool,
        last: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSampledChannel {
    pub channel: usize,
    pub name: String,
    pub initial: bool,
    pub transitions: Vec<CaptureTransition>,
    pub waveform: Vec<CaptureWaveformSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSampledWindow {
    pub start_sample: u64,
    pub end_sample: u64,
    pub sample_step: u64,
    pub channels: Vec<CaptureSampledChannel>,
}

/// Result of a non-blocking sampled-window query.
///
/// Local indexes normally return [`Self::Ready`] immediately. A host-backed
/// index may enqueue bounded work and return [`Self::Pending`], then publish
/// the result from a later call with the same query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureSampledWindowPoll {
    Pending,
    Ready(CaptureSampledWindow),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureFingerprint {
    /// Stable revision used to invalidate persistent indexes.
    ///
    /// File sources can use the file size or a stronger hash/mtime combination.
    /// Live sources normally use their growing index rather than a finite-source index.
    pub revision: u64,
}

pub trait CaptureSource {
    fn metadata(&self) -> &CaptureMetadata;

    fn read_sample(&mut self, channel: usize, position: u64) -> Result<bool>;

    fn capture_duration_us(&self) -> f64 {
        self.metadata().duration_us()
    }

    fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        let metadata = self.metadata().clone();
        let start_sample = start_sample.min(metadata.total_samples.saturating_sub(1));
        let end_sample = end_sample.clamp(start_sample + 1, metadata.total_samples);
        let samples = end_sample - start_sample;
        let target_points = target_points.max(1) as u64;
        let sample_step = samples.div_ceil(target_points).max(1);

        let mut sampled_channels = Vec::with_capacity(channels.len());
        for &channel in channels {
            if channel >= metadata.total_probes {
                return Err(crate::Error::InvalidProbe(channel));
            }

            let name = metadata
                .probe_names
                .get(channel)
                .cloned()
                .unwrap_or_else(|| format!("Probe{channel}"));
            let mut current = self.read_sample(channel, start_sample)?;
            let initial = current;
            let mut transitions = Vec::new();
            let mut sample = start_sample.saturating_add(sample_step);

            while sample < end_sample {
                let value = self.read_sample(channel, sample)?;
                if value != current {
                    current = value;
                    transitions.push(CaptureTransition { sample, value });
                }
                sample = sample.saturating_add(sample_step);
                if sample == u64::MAX {
                    break;
                }
            }

            sampled_channels.push(CaptureSampledChannel {
                channel,
                name,
                initial,
                transitions,
                waveform: Vec::new(),
            });
        }

        Ok(CaptureSampledWindow {
            start_sample,
            end_sample,
            sample_step,
            channels: sampled_channels,
        })
    }
}

/// Shared packed bytes with a zero-copy visible range.
///
/// Fresh decompression uses `From<Vec<u8>>`: the vector moves behind an
/// `Arc` without reallocating or copying its payload. Existing shared slices
/// and memory maps remain valid backing types. Clones and `slice()` views only
/// clone the backing `Arc`.
#[derive(Clone)]
pub struct BlockData {
    region: ByteRegion,
}

struct OwnedBlockRegion(Vec<u8>);

impl ImmutableByteRegion for OwnedBlockRegion {
    fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl BlockData {
    pub fn from_region(region: ByteRegion) -> Self {
        Self { region }
    }

    /// Creates a view into this backing allocation without copying bytes.
    pub fn slice(&self, offset: usize, len: usize) -> Option<Self> {
        let end = offset.checked_add(len)?;
        let length = usize::try_from(self.region.range().length).ok()?;
        if end > length {
            return None;
        }
        let absolute_offset = self.region.range().offset.checked_add(offset as u64)?;
        let range = ByteRange::new(absolute_offset, len as u64).ok()?;
        ByteRegion::new(self.region.clone_backing(), range)
            .ok()
            .map(Self::from_region)
    }

    pub fn shares_backing(&self, other: &Self) -> bool {
        self.region.shares_backing(&other.region)
    }
}

impl std::fmt::Debug for BlockData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockData")
            .field("offset", &self.region.range().offset)
            .field("len", &self.region.range().length)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for BlockData {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.region.bytes()
    }
}

impl From<Arc<[u8]>> for BlockData {
    fn from(data: Arc<[u8]>) -> Self {
        let backing: Arc<dyn ImmutableByteRegion> = Arc::new(crate::OwnedByteSource::new(
            SourceIdentity::from_bytes([0; 32]),
            data,
        ));
        let range = ByteRange::new(0, backing.len()).expect("resident byte length fits u64");
        Self::from_region(ByteRegion::new(backing, range).expect("full backing range is valid"))
    }
}

impl From<Vec<u8>> for BlockData {
    fn from(data: Vec<u8>) -> Self {
        let backing: Arc<dyn ImmutableByteRegion> = Arc::new(OwnedBlockRegion(data));
        let range = ByteRange::new(0, backing.len()).expect("resident byte length fits u64");
        Self::from_region(ByteRegion::new(backing, range).expect("full backing range is valid"))
    }
}

pub trait BlockCaptureSource: CaptureSource {
    fn read_packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData>;
}

/// Reloadable provider for capture data.
///
/// File formats, live captures, and generated/test data should implement this
/// boundary. The indexer only uses this trait; it does not know how the source
/// is opened, reloaded, or backed.
pub trait CaptureDataSource: Clone + Send + Sync + 'static {
    type Reader: BlockCaptureSource + Send + 'static;

    /// Open a fresh reader for the current source revision.
    ///
    /// For finite files this usually opens the file. For live sources this can
    /// return a reader over the latest immutable snapshot or a reloadable
    /// source-specific view.
    fn open_reader(&self) -> Result<Self::Reader>;
    fn metadata(&self) -> &CaptureMetadata;
    fn fingerprint(&self) -> CaptureFingerprint;
    fn index_identity(&self) -> Option<SourceIdentity>;
    fn display_name(&self) -> String;
}

/// Windowed access to an already-opened capture's sample data.
///
/// Implementations may read an in-process index or proxy bounded queries to a
/// host-owned index. Consumers use [`CaptureIndex::poll_sampled_window`] when
/// they must remain responsive while the backing executes independently.
pub trait CaptureIndex {
    fn display_name(&self) -> String;
    fn index_identity(&self) -> SourceIdentity;
    fn header(&self) -> &CaptureMetadata;
    /// Current metadata snapshot. Finite indexes inherit the immutable
    /// header; growing indexes override this with their committed extent.
    fn current_metadata(&self) -> CaptureMetadata {
        self.header().clone()
    }
    /// Monotonic content generation used by viewers to invalidate a sampled
    /// window without polling or identifying a concrete index type.
    fn generation(&self) -> u64 {
        0
    }
    /// Whether no later generation can arrive.
    fn is_complete(&self) -> bool {
        true
    }
    fn capture_duration_us(&self) -> f64;
    fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow>;

    /// Returns one packed raw capture block when this index retains replay access.
    ///
    /// Viewer-only or growing indexes may return `None`. Host-backed file
    /// sessions use this optional capability to stream bounded blocks without
    /// reopening or copying the complete capture.
    fn packed_block(&mut self, _channel: usize, _block: u64) -> Result<Option<BlockData>> {
        Ok(None)
    }

    /// Polls a sampled window without requiring the backing host to block.
    ///
    /// The default preserves immediate local index behavior. Remote indexes
    /// override this method to deduplicate an outstanding query and return
    /// [`CaptureSampledWindowPoll::Pending`] until its bounded result arrives.
    fn poll_sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindowPoll> {
        self.sampled_window(channels, start_sample, end_sample, target_points)
            .map(CaptureSampledWindowPoll::Ready)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureIndexBuildProgress {
    pub completed: u64,
    pub total: u64,
}

/// One bounded result from resumable capture-index preparation.
pub enum CaptureIndexOpenStep {
    Progress(CaptureIndexBuildProgress),
    Ready(Box<dyn CaptureIndex + Send>),
}

/// Resumable capture-index construction owned by a host executor.
pub trait CaptureIndexOpenTask: Send + 'static {
    /// Returns the canonical identity expected from the completed index.
    ///
    /// Incremental builders should expose this before construction completes
    /// so hosts can reject an index assembled for different source data. The
    /// compatibility task returns `None` because legacy synchronous factories
    /// do not declare an identity until they produce the index.
    fn expected_index_identity(&self) -> Option<SourceIdentity> {
        None
    }

    fn step(&mut self) -> Result<CaptureIndexOpenStep>;
}

/// Generic indexed-capture presentation supplied by a concrete source.
///
pub struct IndexedCapturePresentation {
    pub identity: SourceIdentity,
    pub factory: Box<dyn CaptureIndexFactory>,
}

/// Deferred construction of an indexed capture view.
///
/// Concrete file formats implement this at their owning integration boundary. Consumers can move
/// the factory to a worker without knowing the format or opening files on the UI thread.
pub trait CaptureIndexFactory: Send + 'static {
    fn display_name(&self) -> String;

    /// Returns an opaque host-preparation request when the source backing
    /// cannot be opened in the caller's execution context.
    ///
    /// Generic consumers submit this request through their injected host
    /// executor and must not call [`Self::metadata`] or [`Self::open`] for the
    /// same factory.
    fn preparation_request(&self) -> Option<CaptureIndexPreparationRequest> {
        None
    }

    /// Inspects the capture before index construction so hosts can publish
    /// channel and time-span metadata while the index is still being built.
    fn metadata(&self) -> Result<CaptureMetadata>;

    fn open(
        self: Box<Self>,
        artifact_repository: Arc<dyn crate::ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress) -> bool,
    ) -> Result<Box<dyn CaptureIndex + Send>>;

    /// Creates a resumable preparation task.
    ///
    /// The compatibility implementation performs the existing synchronous
    /// open on its first step. Capture formats with block-addressable inputs
    /// override this to yield at deterministic index boundaries.
    fn open_task(
        self: Box<Self>,
        artifact_repository: Arc<dyn crate::ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<Box<dyn CaptureIndexOpenTask>> {
        Ok(Box::new(BlockingCaptureIndexOpenTask {
            factory: Some(self),
            artifact_repository,
            work_executor,
            progress: VecDeque::new(),
            ready: None,
        }))
    }
}

struct BlockingCaptureIndexOpenTask<F: CaptureIndexFactory + ?Sized> {
    factory: Option<Box<F>>,
    artifact_repository: Arc<dyn crate::ArtifactRepository>,
    work_executor: Arc<dyn WorkExecutor>,
    progress: VecDeque<CaptureIndexBuildProgress>,
    ready: Option<Box<dyn CaptureIndex + Send>>,
}

impl<F> CaptureIndexOpenTask for BlockingCaptureIndexOpenTask<F>
where
    F: CaptureIndexFactory + ?Sized,
{
    fn step(&mut self) -> Result<CaptureIndexOpenStep> {
        if let Some(progress) = self.progress.pop_front() {
            return Ok(CaptureIndexOpenStep::Progress(progress));
        }
        if let Some(index) = self.ready.take() {
            return Ok(CaptureIndexOpenStep::Ready(index));
        }
        let factory = self.factory.take().ok_or_else(|| {
            Error::ParseError("capture-index task is already complete".to_owned())
        })?;
        let index = factory.open(
            Arc::clone(&self.artifact_repository),
            Arc::clone(&self.work_executor),
            &mut |progress| {
                self.progress.push_back(progress);
                true
            },
        )?;
        self.ready = Some(index);
        self.step()
    }
}

pub fn packed_bit(data: &[u8], bit_index: usize) -> bool {
    let byte_index = bit_index / 8;
    let bit_offset = bit_index % 8;
    data.get(byte_index)
        .is_some_and(|byte| (byte & (1 << bit_offset)) != 0)
}

#[cfg(test)]
mod tests {
    use super::BlockData;

    #[test]
    fn owned_block_adopts_vec_allocation_and_slices_share_it() {
        let bytes = vec![10, 20, 30, 40, 50];
        let allocation = bytes.as_ptr();
        let data = BlockData::from(bytes);
        assert_eq!(data.as_ptr(), allocation, "Vec payload must not be copied");

        let view = data.slice(1, 3).unwrap();
        assert_eq!(&*view, &[20, 30, 40]);
        assert!(data.shares_backing(&view));
        assert!(data.slice(4, 2).is_none());

        let shared: std::sync::Arc<[u8]> = std::sync::Arc::from([1, 2, 3]);
        assert!(
            BlockData::from(shared.clone()).shares_backing(&BlockData::from(shared)),
            "separate views over one shared slice must identify their backing"
        );
    }
}
