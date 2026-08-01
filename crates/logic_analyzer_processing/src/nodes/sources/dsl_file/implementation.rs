//! DSL file source.
//!
//! Provides `DslFileSource` - a runtime process node that reads DSLogic .dsl capture files
//! and outputs Sample streams per channel (run-length encoded for efficiency).
//!
//! Each broadcast destination runs in its own independently scheduled reader, so a slow consumer
//! on one destination never blocks other destinations. All readers share one capture archive
//! and block cache via `Arc<Mutex<..>>`.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tracing::{debug, info, warn};

use signal_processing::capture::{BlockData, CaptureMetadata, CaptureTransition};
use signal_processing::waveform_index::IndexSampler;
use signal_processing::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, EdgeQuery, Error,
    InlineWorkExecutor, InputPort, OutputPort, ProcessNode, ProtocolKind, Result, Sample,
    SampleBlock, SampleKind, Sender, WorkExecutor, WorkResult, WorkTask,
};

use crate::support::capture_archive::{CaptureArchive, ZipCaptureArchive};
use crate::support::capture_format::get_packed_bit;
use crate::support::capture_index::capture_cache_identity;
use crate::support::dsl_file::{DslChunkedCaptureReader, DslFileCaptureDataSource, parse_header};
const DEFAULT_BLOCK_CACHE_WINDOWS: usize = 2;

type BlockKey = (usize, u64);
type BlockCache = Arc<Mutex<BoundedBlockCache>>;
type SharedCaptureArchive = Arc<Mutex<Box<dyn CaptureArchive>>>;

struct BoundedBlockCache {
    entries: HashMap<BlockKey, BlockData>,
    order: VecDeque<BlockKey>,
    max_entries: usize,
}

impl BoundedBlockCache {
    fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            max_entries: max_entries.max(1),
        }
    }

    fn get(&mut self, key: BlockKey) -> Option<BlockData> {
        let data = self.entries.get(&key)?.clone();
        self.touch(key);
        Some(data)
    }

    fn insert(&mut self, key: BlockKey, data: BlockData) {
        self.entries.insert(key, data);
        self.touch(key);
        while self.entries.len() > self.max_entries {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    fn touch(&mut self, key: BlockKey) {
        self.order.retain(|existing| *existing != key);
        self.order.push_back(key);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// File-backed [`EdgeQuery`] for one channel, sharing the same on-disk
/// `.idx`/`.raw` waveform index the viewer uses for random-access reads
/// (via [`DslChunkedCaptureReader`]).
struct DslChannelEdgeIndex {
    sampler: Arc<Mutex<DslChunkedCaptureReader>>,
    channel: usize,
    sample_period: f64,
    samplerate_hz: f64,
    total_samples: u64,
}

struct DslCaptureIndexFactory {
    path: PathBuf,
}

impl CaptureIndexFactory for DslCaptureIndexFactory {
    fn display_name(&self) -> String {
        self.path.display().to_string()
    }

    fn open(
        self: Box<Self>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: &mut dyn FnMut(CaptureIndexBuildProgress),
    ) -> Result<Box<dyn CaptureIndex + Send>> {
        let source = DslFileCaptureDataSource::open(&self.path)?;
        IndexSampler::open_data_source_with_executor_and_progress(source, work_executor, |value| {
            progress(CaptureIndexBuildProgress {
                completed: value.completed_roots,
                total: value.total_roots,
            });
        })
        .map(|index| Box::new(index) as Box<dyn CaptureIndex + Send>)
    }
}

impl EdgeQuery for DslChannelEdgeIndex {
    fn sample_period(&self) -> f64 {
        self.sample_period
    }

    fn samplerate_hz(&self) -> f64 {
        self.samplerate_hz
    }

    fn total_samples(&self) -> u64 {
        self.total_samples
    }

    fn activity_ratio_hint(&self) -> Option<f64> {
        let sampler = self.sampler.lock().ok()?;
        sampler
            .activity_ratio_hint(self.channel, self.total_samples)
            .ok()
    }

    fn high_level_ratio_hint(&self) -> Option<f64> {
        let sampler = self.sampler.lock().ok()?;
        sampler
            .high_level_ratio_hint(self.channel, self.total_samples)
            .ok()
    }

    fn value_at(&self, position: u64) -> Result<bool> {
        let mut sampler = self.sampler.lock().unwrap();
        sampler.value_at(self.channel, position)
    }

    fn next_edge(&self, position: u64, limit: u64) -> Result<Option<CaptureTransition>> {
        let mut sampler = self.sampler.lock().unwrap();
        sampler.next_transition(self.channel, position, limit.min(self.total_samples))
    }

    fn next_edges(
        &self,
        position: u64,
        limit: u64,
        max_edges: usize,
        output: &mut Vec<CaptureTransition>,
    ) -> Result<()> {
        let mut sampler = self.sampler.lock().unwrap();
        sampler.next_transitions(
            self.channel,
            position,
            limit.min(self.total_samples),
            max_edges,
            output,
        )
    }

    fn values_at(&self, positions: &[u64], output: &mut Vec<bool>) -> Result<()> {
        let mut sampler = self.sampler.lock().unwrap();
        sampler.values_at(self.channel, positions, output)
    }
}

/// Source node that reads from a DSLogic .dsl capture file and outputs Sample streams
///
/// This runtime `ProcessNode` (with 0 inputs, N outputs) reads from a .dsl file and outputs
/// Sample streams for each channel (run-length encoded for efficiency).
///
/// ## Reader execution model
///
/// This is a **self-scheduled node** (`is_self_threading() = true`). On the first (and only)
/// call to `work()`, it submits one internal reader task **per broadcast destination** to its
/// injected host executor.
/// The scheduler thread then waits for `should_stop()` to signal completion, rather than
/// calling `work()` repeatedly.
///
/// If a channel is broadcast to multiple receivers, each receiver gets its own independent
/// reader task. This eliminates head-of-line blocking: slow consumers don't block fast ones.
/// All readers share a single capture archive and block cache via `Arc<Mutex<..>>`.
///
/// Example: If channel 0 connects to both `spi_decoder` and `parallel_decoder`, two readers
/// are submitted:
/// - Reader 1: reads channel 0 data → sends to `spi_decoder`
/// - Reader 2: reads channel 0 data → sends to `parallel_decoder`
///
/// If `parallel_decoder` blocks (waiting for enable signal), Reader 2 blocks but Reader 1
/// continues, ensuring `spi_decoder` receives data without interruption.
///
/// # Features
/// - Opens and parses DSLogic capture files (.dsl format)
/// - Per-destination reader tasks eliminate head-of-line blocking
/// - On-demand block loading with shared caching for efficiency
/// - Automatic timestamp generation based on sample rate
/// - Sample output (only sends on signal transitions)
/// - Exposes every channel declared by the capture
///
/// # Example
/// ```ignore
/// let source = DslFileSource::new("capture.dsl")?;
/// let handle = pipeline.add_process(source);
/// ```
pub struct DslFileSource {
    name: String,
    // File access shared across all channel readers.
    path: PathBuf,
    archive: SharedCaptureArchive,
    header: CaptureMetadata,
    blocks: BlockCache,

    // Configuration
    num_channels: usize,
    max_samples: Option<u64>,

    // Per-destination reader task management.
    shutdown: Arc<AtomicBool>,
    threads_completed: Arc<AtomicUsize>,
    reader_tasks: Option<Vec<Box<dyn WorkTask>>>,
    threads_spawned: bool,
    num_threads: usize,
    work_executor: Arc<dyn WorkExecutor>,

    // Lazily-built random-access waveform index, shared across every
    // channel's `edge_query()` handle. Built at most once, only if a
    // downstream node actually negotiates the `EdgeQuery` protocol — see
    // `edge_index_handle`.
    index: Mutex<Option<Arc<Mutex<DslChunkedCaptureReader>>>>,
}

impl DslFileSource {
    /// Creates the generic indexed-capture presentation for a static DSL file.
    pub fn indexed_capture_presentation(
        path: impl AsRef<Path>,
    ) -> signal_processing::IndexedCapturePresentation {
        let path = path.as_ref().to_path_buf();
        signal_processing::IndexedCapturePresentation {
            identity: path.clone(),
            factory: Box::new(DslCaptureIndexFactory { path }),
        }
    }

    /// Returns the persistent-cache identity for a static DSL file.
    pub fn capture_cache_identity(path: impl AsRef<Path>) -> Result<[u8; 32]> {
        let path = path.as_ref();
        let source = DslFileCaptureDataSource::open(path)?;
        Ok(capture_cache_identity(path, &source))
    }

    /// Create a new DSL file source from a file path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let archive = Box::new(ZipCaptureArchive::open(&path)?);
        Self::from_archive(path, archive)
    }

    fn from_archive(path: PathBuf, mut archive: Box<dyn CaptureArchive>) -> Result<Self> {
        let header = parse_header(archive.as_mut())?;

        let num_channels = header.total_probes;

        Ok(Self {
            name: "dsl_file_source".to_string(),
            path,
            archive: Arc::new(Mutex::new(archive)),
            header: header.clone(),
            blocks: Arc::new(Mutex::new(BoundedBlockCache::new(
                num_channels * DEFAULT_BLOCK_CACHE_WINDOWS,
            ))),
            num_channels,
            max_samples: None,
            shutdown: Arc::new(AtomicBool::new(false)),
            threads_completed: Arc::new(AtomicUsize::new(0)),
            reader_tasks: None,
            threads_spawned: false,
            num_threads: 0,
            work_executor: Arc::new(InlineWorkExecutor),
            index: Mutex::new(None),
        })
    }

    /// Random-access handle backing `edge_query()`, built on first use from
    /// the same `.idx`/`.raw` sidecar files the viewer uses (via
    /// `DslFileCaptureDataSource`/`IndexSampler`). Returns `None` (logging a
    /// warning) if the index can't be built — callers fall back to `Stream`.
    fn edge_index_handle(&self) -> Option<Arc<Mutex<DslChunkedCaptureReader>>> {
        let mut guard = self.index.lock().unwrap();
        if guard.is_none() {
            let source = match DslFileCaptureDataSource::open(&self.path) {
                Ok(source) => source,
                Err(e) => {
                    warn!("Failed to open capture for edge queries: {}", e);
                    return None;
                }
            };
            match IndexSampler::open_data_source_with_progress(source, |_| {}) {
                Ok(sampler) => *guard = Some(Arc::new(Mutex::new(sampler))),
                Err(e) => {
                    warn!("Failed to build waveform index for edge queries: {}", e);
                    return None;
                }
            }
        }
        guard.clone()
    }
    /// Get the header information
    pub fn header(&self) -> &CaptureMetadata {
        &self.header
    }

    /// Get the total number of probes
    pub fn total_probes(&self) -> usize {
        self.header.total_probes
    }

    /// Get the total number of samples
    pub fn total_samples(&self) -> u64 {
        self.header.total_samples
    }

    /// Get the sample rate in Hz
    pub fn samplerate_hz(&self) -> f64 {
        self.header.samplerate_hz
    }

    /// Get the sample period in seconds
    pub fn sample_period(&self) -> f64 {
        self.header.sample_period
    }

    /// Get the total capture duration in seconds
    pub fn capture_duration(&self) -> f64 {
        self.header.total_samples as f64 * self.header.sample_period
    }

    /// Read a single bit from a specific channel at a specific position
    pub fn read_bit(&self, channel: usize, position: u64) -> Result<bool> {
        if channel >= self.header.total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        if position >= self.header.total_samples {
            return Err(Error::OutOfBounds(position));
        }

        let block_num = position / self.header.samples_per_block;

        // Additional safety check: ensure block number is valid
        if block_num >= self.header.total_blocks {
            return Err(Error::OutOfBounds(position));
        }

        let sample_in_block = (position % self.header.samples_per_block) as usize;

        let data = Self::load_block(&self.archive, &self.blocks, channel, block_num)?;
        let result = get_packed_bit(&data, sample_in_block);
        Ok(result)
    }

    /// Set custom name (builder pattern)
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Selects the host executor used for independently scheduled readers.
    pub fn with_work_executor(mut self, work_executor: Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = work_executor;
        self
    }

    /// Set maximum number of samples to read from file (for benchmarking)
    pub fn with_max_samples(mut self, max_samples: Option<u64>) -> Self {
        self.max_samples = max_samples;
        self
    }

    /// Get the number of channels this source outputs
    pub fn num_channels(&self) -> usize {
        self.num_channels
    }

    // ── Associated Functions (Helpers) ──────────────────────────────────
    fn load_block(
        archive: &SharedCaptureArchive,
        blocks: &BlockCache,
        channel: usize,
        block_num: u64,
    ) -> Result<BlockData> {
        let key = (channel, block_num);
        if let Some(data) = blocks.lock().unwrap().get(key) {
            return Ok(data);
        }

        // Archive access is serialized. Recheck the cache after obtaining
        // that lock because another destination may have decompressed this
        // block while this reader was waiting.
        let mut archive_guard = archive.lock().unwrap();
        if let Some(data) = blocks.lock().unwrap().get(key) {
            return Ok(data);
        }
        let block_name = format!("L-{channel}/{block_num}");
        let bytes = archive_guard
            .read_entry(&block_name)?
            .ok_or(Error::InvalidBlock(block_num))?;
        let data = BlockData::from(bytes);
        blocks.lock().unwrap().insert(key, data.clone());
        Ok(data)
    }

    /// Reader task that sends one channel's data to one destination.
    ///
    /// Each task loads blocks from the shared archive + cache, walks bits
    /// to detect edges, and sends Samples to its destination. Tasks are
    /// fully independent — if a channel is broadcast to multiple destinations,
    /// each destination gets its own reader task reading the same channel data.
    /// This eliminates head-of-line blocking: slow destinations don't block fast ones.
    ///
    /// Cross-channel temporal alignment is handled by downstream decoders
    /// using timestamps (e.g., `drain_before()` and `value_at_time()`).
    fn channel_reader_thread(config: ChannelReaderConfig) {
        let ChannelReaderConfig {
            archive,
            blocks,
            channel,
            header,
            sender,
            destination,
            max_samples,
            shutdown,
            completed,
        } = config;
        let label = channel_log_label(channel, destination.as_deref());
        let timestamp_step = (1_000_000_000.0 / header.samplerate_hz) as u64;
        let total_samples = max_samples
            .unwrap_or(header.total_samples)
            .min(header.total_samples);

        let mut current_value = false;
        let mut value_start_time: u64 = 0;
        let mut position: u64 = 0;
        let mut items_sent: u64 = 0;

        info!(
            "[{}] Starting channel reader ({} samples, {} blocks)",
            label, total_samples, header.total_blocks
        );

        for block_num in 0..header.total_blocks {
            if shutdown.load(Ordering::Relaxed) {
                debug!(
                    "[{}] Shutdown signal received at block {}",
                    label, block_num
                );
                break;
            }

            // Check if we've exceeded our sample limit
            let block_start_position = block_num * header.samples_per_block;
            if block_start_position >= total_samples {
                break;
            }

            let block_data = match Self::load_block(&archive, &blocks, channel, block_num) {
                Ok(data) => data,
                Err(error) => {
                    debug!("[{}] Failed to read block {}: {}", label, block_num, error);
                    break;
                }
            };

            // Walk bits in this block, detecting edges
            let block_capacity = (block_data.len() * 8) as u64;
            let samples_in_block = block_capacity.min(total_samples - block_start_position);

            for sample_in_block in 0..samples_in_block as usize {
                let value = get_packed_bit(&block_data, sample_in_block);
                let timestamp = position * timestamp_step;

                if position == 0 {
                    current_value = value;
                    value_start_time = timestamp;
                } else if value != current_value {
                    let edge = Sample::new(current_value, value_start_time);
                    if sender.send(edge).is_err() {
                        debug!(
                            "[{}] All receivers disconnected at position {}",
                            label, position
                        );
                        completed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    items_sent += 1;

                    current_value = value;
                    value_start_time = timestamp;
                }

                position += 1;
            }

            if block_num > 0 && block_num % 10 == 0 {
                let pct = (position as f64 / total_samples as f64) * 100.0;
                debug!(
                    "[{}] Progress: {:.1}% ({} samples, {} edges sent)",
                    label, pct, position, items_sent
                );
            }
        }

        // Send final edge for the last value
        if position > 0 {
            let final_edge = Sample::new(current_value, value_start_time);
            let _ = sender.send(final_edge);
            items_sent += 1;
        }

        info!(
            "[{}] Channel reader complete: {} samples, {} edges sent",
            label, position, items_sent
        );

        sender.close();
        drop(sender);
        completed.fetch_add(1, Ordering::Relaxed);
    }

    /// Sends aligned packed blocks to one destination node. All connected
    /// channels for block N are delivered before this worker advances to
    /// block N+1, while unrelated destinations keep independent workers.
    fn block_reader_thread(config: BlockReaderGroupConfig) {
        let BlockReaderGroupConfig {
            archive,
            blocks,
            indexed_blocks,
            destinations,
            group_label,
            header,
            max_samples,
            shutdown,
            completed,
        } = config;
        let timestamp_step = (1_000_000_000.0 / header.samplerate_hz) as u64;
        let total_samples = max_samples
            .unwrap_or(header.total_samples)
            .min(header.total_samples);

        info!(
            "[{}] Starting aligned block reader ({} channels, {} samples, {} blocks)",
            group_label,
            destinations.len(),
            total_samples,
            header.total_blocks
        );

        'blocks: for block_num in 0..header.total_blocks {
            if shutdown.load(Ordering::Relaxed) {
                debug!(
                    "[{}] Block reader shutdown at block {}",
                    group_label, block_num
                );
                break;
            }

            let block_start_position = block_num * header.samples_per_block;
            if block_start_position >= total_samples {
                break;
            }

            let mut last_samples_in_block = 0usize;
            for destination in &destinations {
                let block_data = match indexed_blocks.as_ref() {
                    Some(index) => index
                        .lock()
                        .unwrap()
                        .packed_block(destination.channel, block_num),
                    None => Self::load_block(&archive, &blocks, destination.channel, block_num),
                };
                let block_data = match block_data {
                    Ok(data) => data,
                    Err(error) => {
                        debug!(
                            "[{}] Failed to read channel {} block {}: {}",
                            group_label, destination.channel, block_num, error
                        );
                        break 'blocks;
                    }
                };
                let block_capacity = (block_data.len() * 8) as u64;
                let samples_in_block =
                    block_capacity.min(total_samples - block_start_position) as usize;
                last_samples_in_block = samples_in_block;
                let sample_block = SampleBlock::new(
                    block_data,
                    block_start_position,
                    samples_in_block,
                    timestamp_step,
                );

                if destination.sender.send(sample_block).is_err() {
                    debug!(
                        "[{}] Receiver disconnected at channel {} block {}",
                        group_label, destination.channel, block_num
                    );
                    break 'blocks;
                }
            }

            if block_num > 0 && block_num % 10 == 0 {
                let pct = ((block_start_position + last_samples_in_block as u64) as f64
                    / total_samples as f64)
                    * 100.0;
                debug!(
                    "[{}] Block progress: {:.1}% ({} blocks sent)",
                    group_label,
                    pct,
                    block_num + 1
                );
            }
        }

        info!("[{}] Block reader complete", group_label);

        for destination in destinations {
            destination.sender.close();
        }
        completed.fetch_add(1, Ordering::Relaxed);
    }
}

impl ProcessNode for DslFileSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn should_stop(&self) -> bool {
        self.threads_spawned && self.threads_completed.load(Ordering::Relaxed) >= self.num_threads
    }

    fn is_self_threading(&self) -> bool {
        true
    }

    fn num_inputs(&self) -> usize {
        0 // Source node
    }

    fn num_outputs(&self) -> usize {
        // One port per channel, `ch0..chN` — negotiates Sample vs
        // SampleBlock per connection (see `output_schema`'s
        // `with_sample_kinds`) instead of exposing separate `d`/`b` ports
        // for each.
        self.num_channels
    }

    fn output_schema(&self) -> Vec<signal_processing::PortSchema> {
        use signal_processing::{PortDirection, PortSchema};

        (0..self.num_channels)
            .map(|i| {
                PortSchema::new::<Sample>(format!("ch{}", i), i, PortDirection::Output)
                    // Every channel port aliases a raw file channel, so
                    // every port can be answered from the waveform index —
                    // prefer that, fall back to streaming for consumers (or
                    // live sources with no index) that don't ask for it.
                    .with_protocols(vec![ProtocolKind::EdgeQuery, ProtocolKind::Stream])
                    // Block is a near-zero-cost passthrough of the on-disk
                    // block; Edge costs a real bit-walk to derive RLE edges
                    // (see `block_reader_thread`/`channel_reader_thread`
                    // below) — prefer Block, but a consumer that only wants
                    // Edge still gets it.
                    .with_sample_kinds(vec![SampleKind::Block, SampleKind::Edge])
            })
            .collect()
    }

    fn edge_query(
        &self,
        port: usize,
        _input_queries: &[Option<Arc<dyn EdgeQuery>>],
    ) -> Option<Arc<dyn EdgeQuery>> {
        let channel = port;
        let sampler = self.edge_index_handle()?;
        // Honor `with_max_samples` the same way the streaming reader
        // readers do, so a bounded source behaves identically regardless
        // of which protocol a connection negotiates.
        let total_samples = self
            .max_samples
            .unwrap_or(self.header.total_samples)
            .min(self.header.total_samples);
        Some(Arc::new(DslChannelEdgeIndex {
            sampler,
            channel,
            sample_period: self.header.sample_period,
            samplerate_hz: self.header.samplerate_hz,
            total_samples,
        }))
    }

    fn work(&mut self, _inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize> {
        use signal_processing::WorkError;

        if self.threads_spawned {
            // Already started - this shouldn't be called again for self-threading nodes
            return Err(WorkError::NodeError(
                "work() called multiple times on self-threading node".to_string(),
            ));
        }

        // First and only call: submit one reader per connected output destination.
        self.threads_spawned = true;

        info!(
            "File source: Scheduling per-destination readers for {} samples at {:.1} MHz ({} channels)",
            self.header.total_samples,
            self.header.samplerate_hz / 1_000_000.0,
            self.num_channels
        );

        // Collect all channel-destination pairs to schedule readers for.
        // Each destination gets its own independent reader task. Every
        // channel has exactly one output port now, but that port can
        // carry `Sample` and `SampleBlock` destinations simultaneously
        // (negotiated per connection — see `output_sample_kinds`), so
        // both queries run independently against the same port.
        let mut edge_reader_configs: Vec<(usize, usize, Sender<Sample>, Option<String>)> =
            Vec::new();
        let mut block_reader_groups: HashMap<String, Vec<BlockDestination>> = HashMap::new();

        for channel_idx in 0..self.num_channels {
            let Some(port) = outputs.get(channel_idx) else {
                continue;
            };
            if let Some(senders) = port.split_senders::<Sample>() {
                for (dest_idx, sender) in senders.into_iter().enumerate() {
                    let destination = sender.destination_label().map(str::to_owned);
                    edge_reader_configs.push((channel_idx, dest_idx, sender, destination));
                }
            }
            if let Some(senders) = port.split_senders::<SampleBlock>() {
                for (dest_idx, sender) in senders.into_iter().enumerate() {
                    let destination = sender.destination_label().map(str::to_owned);
                    let group =
                        block_destination_group(channel_idx, dest_idx, destination.as_deref());
                    block_reader_groups
                        .entry(group)
                        .or_default()
                        .push(BlockDestination {
                            channel: channel_idx,
                            sender,
                        });
                }
            }
        }

        let mut reader_tasks = Vec::new();

        // Schedule edge readers.
        for (channel_idx, _dest_idx, sender, destination) in edge_reader_configs {
            let archive = Arc::clone(&self.archive);
            let blocks = Arc::clone(&self.blocks);
            let header = self.header.clone();
            let max_samples = self.max_samples;
            let shutdown = Arc::clone(&self.shutdown);
            let completed = Arc::clone(&self.threads_completed);

            let task = self
                .work_executor
                .submit_long_running(Box::new(move || {
                    Self::channel_reader_thread(ChannelReaderConfig {
                        archive,
                        blocks,
                        channel: channel_idx,
                        header,
                        sender,
                        destination,
                        max_samples,
                        shutdown,
                        completed,
                    });
                }))
                .map_err(WorkError::NodeError)?;

            reader_tasks.push(task);
        }

        // Each destination node receives all of its raw channels from one
        // block-major worker. Different destinations remain independent.
        for (group_label, mut destinations) in block_reader_groups {
            destinations.sort_by_key(|destination| destination.channel);
            let archive = Arc::clone(&self.archive);
            let blocks = Arc::clone(&self.blocks);
            let indexed_blocks = self.index.lock().unwrap().clone();
            let header = self.header.clone();
            let max_samples = self.max_samples;
            let shutdown = Arc::clone(&self.shutdown);
            let completed = Arc::clone(&self.threads_completed);

            let task = self
                .work_executor
                .submit_long_running(Box::new(move || {
                    Self::block_reader_thread(BlockReaderGroupConfig {
                        archive,
                        blocks,
                        indexed_blocks,
                        destinations,
                        group_label,
                        header,
                        max_samples,
                        shutdown,
                        completed,
                    });
                }))
                .map_err(WorkError::NodeError)?;

            reader_tasks.push(task);
        }

        self.num_threads = reader_tasks.len();
        self.reader_tasks = Some(reader_tasks);

        info!(
            "File source: Scheduled {} reader workers for {} available channels",
            self.num_threads, self.num_channels
        );

        Ok(0)
    }
}

impl Drop for DslFileSource {
    fn drop(&mut self) {
        // Signal all readers to stop.
        self.shutdown.store(true, Ordering::Relaxed);

        // Wait for all host-owned reader tasks.
        if let Some(tasks) = self.reader_tasks.take() {
            for task in tasks {
                task.wait();
            }
        }
    }
}

// ============================================================================
// Per-channel reader task
// ============================================================================

/// Configuration for a per-channel reader task.
struct ChannelReaderConfig {
    archive: SharedCaptureArchive,
    blocks: BlockCache,
    channel: usize,
    header: CaptureMetadata,
    sender: Sender<Sample>,
    destination: Option<String>,
    max_samples: Option<u64>,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

struct BlockDestination {
    channel: usize,
    sender: Sender<SampleBlock>,
}

/// Configuration for one destination node's aligned block reader.
struct BlockReaderGroupConfig {
    archive: SharedCaptureArchive,
    blocks: BlockCache,
    indexed_blocks: Option<Arc<Mutex<DslChunkedCaptureReader>>>,
    destinations: Vec<BlockDestination>,
    group_label: String,
    header: CaptureMetadata,
    max_samples: Option<u64>,
    shutdown: Arc<AtomicBool>,
    completed: Arc<AtomicUsize>,
}

fn channel_log_label(channel: usize, destination: Option<&str>) -> String {
    match destination {
        Some(destination) if !destination.is_empty() => format!("ch{channel} -> {destination}"),
        _ => format!("ch{channel}"),
    }
}

fn block_destination_group(
    channel: usize,
    destination_index: usize,
    destination: Option<&str>,
) -> String {
    destination
        .and_then(|label| label.rsplit_once('.').map(|(node, _)| node.to_string()))
        .unwrap_or_else(|| format!("ch{channel}_dest{destination_index}"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::File;
    use std::io::Write;

    use signal_processing::{
        CompletedWorkTask, OutputPort, ProcessNode, Sender, Watchdog, WorkExecutor,
        WorkExecutorTask, WorkTask,
    };

    use super::*;
    use crate::support::capture_format::{get_packed_bit, parse_sample_rate};
    use crate::support::dsl_file::DslCaptureReader;

    pub(crate) fn open_dsl_chunked_capture<P: AsRef<Path>>(
        path: P,
    ) -> Result<DslChunkedCaptureReader> {
        let source = DslFileCaptureDataSource::open(path)?;
        IndexSampler::open_data_source_with_progress(source, |_| {})
    }

    #[test]
    fn bounded_block_cache_evicts_least_recently_used_entry() {
        let mut cache = BoundedBlockCache::new(2);
        cache.insert((0, 0), BlockData::from(vec![0]));
        cache.insert((1, 0), BlockData::from(vec![1]));
        assert!(cache.get((0, 0)).is_some());
        cache.insert((2, 0), BlockData::from(vec![2]));

        assert_eq!(cache.len(), 2);
        assert!(cache.get((0, 0)).is_some());
        assert!(cache.get((1, 0)).is_none());
        assert!(cache.get((2, 0)).is_some());
    }

    #[test]
    fn block_destinations_group_by_consumer_node() {
        assert_eq!(block_destination_group(0, 0, Some("decoder.d0")), "decoder");
        assert_eq!(
            block_destination_group(10, 0, Some("decoder.strobe")),
            "decoder"
        );
        assert_ne!(
            block_destination_group(0, 0, Some("decoder.d0")),
            block_destination_group(0, 0, Some("viewer.in0"))
        );
        assert_eq!(block_destination_group(3, 2, None), "ch3_dest2");
    }

    #[test]
    fn test_parse_sample_rate_valid() {
        assert_eq!(parse_sample_rate("50 MHz"), Some(50_000_000.0));
        assert_eq!(parse_sample_rate("1 GHz"), Some(1_000_000_000.0));
        assert_eq!(parse_sample_rate("100 kHz"), Some(100_000.0));
        assert_eq!(parse_sample_rate("100 KHz"), Some(100_000.0));
        assert_eq!(parse_sample_rate("1000 Hz"), Some(1000.0));
        assert_eq!(parse_sample_rate("2.5 MHz"), Some(2_500_000.0));
    }

    #[test]
    fn test_parse_sample_rate_invalid() {
        assert_eq!(parse_sample_rate("invalid"), None);
        assert_eq!(parse_sample_rate("50"), None);
        assert_eq!(parse_sample_rate("MHz 50"), None);
        assert_eq!(parse_sample_rate("50 mhz"), None);
        assert_eq!(parse_sample_rate(""), None);
        assert_eq!(parse_sample_rate("abc MHz"), None);
    }

    #[test]
    fn test_get_bit() {
        let data = vec![0b10101010, 0b11001100];
        assert!(!get_packed_bit(&data, 0)); // bit 0 of byte 0
        assert!(get_packed_bit(&data, 1)); // bit 1 of byte 0
        assert!(!get_packed_bit(&data, 2)); // bit 2 of byte 0
        assert!(get_packed_bit(&data, 3)); // bit 3 of byte 0
        assert!(get_packed_bit(&data, 7)); // bit 7 of byte 0
        assert!(!get_packed_bit(&data, 8)); // bit 0 of byte 1
        assert!(!get_packed_bit(&data, 9)); // bit 1 of byte 1
        assert!(get_packed_bit(&data, 10)); // bit 2 of byte 1
        assert!(get_packed_bit(&data, 11)); // bit 3 of byte 1

        // Out of bounds
        assert!(!get_packed_bit(&data, 16));
        assert!(!get_packed_bit(&data, 100));
    }

    #[test]
    fn test_capture_reader_reads_a_window_from_a_generated_fixture() {
        let (_directory, path) = dsl_fixture();

        let mut reader = DslCaptureReader::open(&path)
            .expect("generated DSL capture should open with the windowed reader")
            .with_max_cached_blocks(4);
        assert!(reader.header().total_samples > 0);
        assert!(reader.header().total_probes > 0);

        let channel_count = reader.header().total_probes.min(4);
        let channels: Vec<usize> = (0..channel_count).collect();
        let window = reader
            .sampled_window(&channels, 0, 8, 8)
            .expect("generated DSL capture viewport should read");

        assert_eq!(window.channels.len(), channel_count);
        assert!(window.sample_step > 0);
    }

    #[test]
    fn test_dsl_channel_edge_index_matches_ground_truth() {
        let (_directory, path) = dsl_fixture();

        let source = DslFileSource::new(&path).expect("generated DSL capture should open");
        let edge_query = source
            .edge_query(0, &[])
            .expect("DslFileSource should provide an EdgeQuery for channel 0");

        // Ground truth: exact transitions over a bounded prefix, computed
        // directly against the index (bypassing the galloping wrapper) so
        // this validates next_edge's search logic against real data shape,
        // not just the index itself.
        let ground_truth_end = 2_000_000u64.min(edge_query.total_samples());
        let mut sampler = open_dsl_chunked_capture(&path).expect("sampler should open");
        let window = sampler
            .sampled_window(&[0], 0, ground_truth_end, ground_truth_end as usize)
            .expect("exact window should read");
        let expected: Vec<(u64, bool)> = window.channels[0]
            .transitions
            .iter()
            .map(|t| (t.sample, t.value))
            .collect();

        // Walk next_edge from 0 and confirm it reproduces the same sequence
        // (exercises galloping across whatever gap sizes occur in the real
        // signal, small and large alike).
        let mut position = 0u64;
        let mut found = Vec::new();
        while let Some(t) = edge_query
            .next_edge(position, ground_truth_end)
            .expect("next_edge should not error")
        {
            found.push((t.sample, t.value));
            position = t.sample;
        }
        assert_eq!(found, expected);

        // value_at agrees with the transitions: the new value holds at/after
        // the edge, the old value holds strictly before it.
        for &(sample, value) in &expected {
            assert_eq!(edge_query.value_at(sample).unwrap(), value);
            if sample > 0 {
                assert_ne!(edge_query.value_at(sample - 1).unwrap(), value);
            }
        }
    }

    #[test]
    fn test_dsl_channel_edge_index_next_edge_with_value() {
        let (_directory, path) = dsl_fixture();
        let source = DslFileSource::new(&path).expect("generated DSL capture should open");
        let edge_query = source.edge_query(0, &[]).expect("edge query available");
        let limit = 2_000_000u64.min(edge_query.total_samples());

        let first = edge_query
            .next_edge(0, limit)
            .unwrap()
            .expect("generated fixture must contain a transition");

        let same = edge_query
            .next_edge_with_value(0, first.value, limit)
            .unwrap()
            .expect("the first transition itself satisfies its own value");
        assert_eq!(same, first);

        // Edges alternate, so the opposite value's first occurrence (if any
        // before `limit`) is strictly after `first`.
        let other = edge_query
            .next_edge_with_value(0, !first.value, limit)
            .unwrap()
            .expect("generated fixture must contain the opposite transition value");
        assert_ne!(other.value, first.value);
        assert!(other.sample > first.sample);
    }

    #[test]
    fn test_dsl_channel_edge_index_end_of_file() {
        let (_directory, path) = dsl_fixture();
        let source = DslFileSource::new(&path).expect("generated DSL capture should open");
        let edge_query = source.edge_query(0, &[]).expect("edge query available");
        let total = edge_query.total_samples();

        assert_eq!(edge_query.next_edge(total - 1, total).unwrap(), None);
        assert_eq!(edge_query.next_edge(total, total).unwrap(), None);
    }

    fn dsl_fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.dsl");
        let file = File::create(&path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        archive.start_file("header", options).unwrap();
        archive
            .write_all(
                b"total probes = 8\nsamplerate = 1 MHz\ntotal samples = 1024\ntotal blocks = 1\nprobe0 = D0\nprobe1 = D1\nprobe2 = D2\nprobe3 = D3\nprobe4 = D4\nprobe5 = D5\nprobe6 = D6\nprobe7 = D7\n",
            )
            .unwrap();
        for channel in 0..8 {
            archive
                .start_file(format!("L-{channel}/0"), options)
                .unwrap();
            archive
                .write_all(&[if channel % 2 == 0 { 0xAA } else { 0x55 }; 128])
                .unwrap();
        }
        archive.finish().unwrap();
        (directory, path)
    }

    fn dsl_source() -> DslFileSource {
        DslFileSource::from_archive(
            PathBuf::from("virtual/fixture.dsl"),
            Box::new(TestCaptureArchive::fixture()),
        )
        .unwrap()
    }

    struct RecordingReaderExecutor {
        submitted: AtomicUsize,
    }

    impl WorkExecutor for RecordingReaderExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn submit(
            &self,
            _task: WorkExecutorTask,
        ) -> std::result::Result<Box<dyn WorkTask>, String> {
            Err("finite work must not schedule a file reader".into())
        }

        fn submit_long_running(
            &self,
            task: WorkExecutorTask,
        ) -> std::result::Result<Box<dyn WorkTask>, String> {
            self.submitted.fetch_add(1, Ordering::Relaxed);
            task();
            Ok(Box::new(CompletedWorkTask))
        }
    }

    #[derive(Default)]
    struct TestCaptureArchive {
        entries: BTreeMap<String, Vec<u8>>,
    }

    impl TestCaptureArchive {
        fn fixture() -> Self {
            let mut entries = BTreeMap::from([(
                "header".to_owned(),
                b"total probes = 8\nsamplerate = 1 MHz\ntotal samples = 1024\ntotal blocks = 1\nprobe0 = D0\nprobe1 = D1\nprobe2 = D2\nprobe3 = D3\nprobe4 = D4\nprobe5 = D5\nprobe6 = D6\nprobe7 = D7\n"
                    .to_vec(),
            )]);
            for channel in 0..8 {
                entries.insert(
                    format!("L-{channel}/0"),
                    vec![if channel % 2 == 0 { 0xAA } else { 0x55 }; 128],
                );
            }
            Self { entries }
        }
    }

    impl CaptureArchive for TestCaptureArchive {
        fn entry_names(&self) -> Vec<String> {
            self.entries.keys().cloned().collect()
        }

        fn entry_size(&mut self, name: &str) -> Result<Option<u64>> {
            Ok(self.entries.get(name).map(|entry| entry.len() as u64))
        }

        fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
            Ok(self.entries.get(name).cloned())
        }
    }

    #[test]
    fn test_dsl_file_source_new_valid() {
        let source = dsl_source();
        assert_eq!(source.num_channels(), source.header().total_probes);
        assert_eq!(source.num_inputs(), 0);
        assert_eq!(source.num_outputs(), source.header().total_probes);
        assert_eq!(source.name(), "dsl_file_source");
        let header = source.header();
        assert!(header.total_probes > 0);
        assert!(header.total_samples > 0);
        assert!(header.samplerate_hz > 0.0);
        assert!(header.sample_period > 0.0);
    }

    #[test]
    fn test_dsl_file_source_invalid_file() {
        let result = DslFileSource::new("nonexistent.dsl");
        assert!(result.is_err());
    }

    #[test]
    fn test_dsl_file_source_builder_methods() {
        let source = dsl_source().with_name("custom_source");
        assert_eq!(source.name(), "custom_source");
    }

    #[test]
    fn test_dsl_file_source_getters() {
        let source = dsl_source();
        assert!(source.total_probes() > 0);
        assert!(source.total_samples() > 0);
        assert!(source.samplerate_hz() > 0.0);
        assert!(source.sample_period() > 0.0);
        assert!(source.capture_duration() > 0.0);
        let expected_duration = source.total_samples() as f64 * source.sample_period();
        assert!((source.capture_duration() - expected_duration).abs() < 0.0001);
    }

    #[test]
    fn test_dsl_file_source_worknode_methods() {
        let source = dsl_source();
        assert!(!source.should_stop());
        assert!(!source.threads_spawned);
    }

    #[test]
    fn source_schedules_readers_through_the_injected_executor() {
        let executor = Arc::new(RecordingReaderExecutor {
            submitted: AtomicUsize::new(0),
        });
        let mut source = dsl_source().with_work_executor(executor.clone());
        let watchdog = Watchdog::new();
        let (sender, _receiver) = crossbeam_channel::bounded(2_048);
        let outputs = [OutputPort::new_with_watchdog(
            Sender::<Sample>::new(vec![sender]),
            &watchdog,
            "source",
            "ch0",
        )];

        source.work(&[], &outputs).unwrap();

        assert_eq!(executor.submitted.load(Ordering::Relaxed), 1);
        assert!(source.should_stop());
    }

    #[test]
    fn test_dsl_file_source_read_bit_valid() {
        let source = dsl_source();
        let bit_result = source.read_bit(0, 0);
        assert!(
            bit_result.is_ok(),
            "Failed to read bit: {:?}",
            bit_result.err()
        );
        assert!(source.read_bit(5, 100).is_ok());
    }

    #[test]
    fn test_dsl_file_source_read_bit_invalid_channel() {
        let error = dsl_source().read_bit(99, 0).unwrap_err();
        assert!(matches!(error, Error::InvalidProbe(_)));
    }

    #[test]
    fn test_dsl_file_source_read_bit_invalid_position() {
        let error = dsl_source().read_bit(0, u64::MAX).unwrap_err();
        assert!(matches!(error, Error::OutOfBounds(_)));
    }

    #[test]
    fn test_dsl_file_source_header_fields() {
        let source = dsl_source();
        let header = source.header();
        assert!(header.total_probes >= 8);
        assert!(header.total_samples > 0);
        assert!(header.total_blocks > 0);
        assert!(header.samples_per_block > 0);
        assert!(!header.samplerate.is_empty());
        assert!(header.samplerate_hz > 0.0);
        assert!(header.sample_period > 0.0);
        assert!(header.probe_names.len() == header.total_probes);
        let expected_period = 1.0 / header.samplerate_hz;
        assert!((header.sample_period - expected_period).abs() < 1e-10);
        let average_per_block = header.total_samples / header.total_blocks;
        assert!(header.samples_per_block >= average_per_block);
        assert_eq!(header.samples_per_block, 1024);
    }

    #[test]
    fn test_dsl_file_source_block_caching() {
        let source = dsl_source();
        let bit1 = source.read_bit(0, 0);
        let bit2 = source.read_bit(0, 0);
        assert!(bit1.is_ok());
        assert_eq!(bit1.unwrap(), bit2.unwrap());
        let cache = source.blocks.lock().unwrap();
        assert!(!cache.is_empty(), "Cache should not be empty after reads");
    }

    #[test]
    fn test_dsl_file_source_multiple_channels() {
        let source = dsl_source();
        let mut channel_values = Vec::new();
        for channel in 0..8 {
            let bit_result = source.read_bit(channel, 1000);
            assert!(
                bit_result.is_ok(),
                "Failed to read channel {channel}: {:?}",
                bit_result.err()
            );
            channel_values.push(bit_result.unwrap());
        }
        assert_eq!(channel_values.len(), 8);
    }
}
