//! DSL file source.
//!
//! Provides `DslFileSource` - a runtime process node that reads DSLogic .dsl capture files
//! and outputs Sample streams per channel (run-length encoded for efficiency).
//!
//! Each broadcast destination runs in its own independent reading thread, so a slow consumer
//! on one destination never blocks other destinations. All threads share a single ZipArchive
//! and block cache via `Arc<Mutex<..>>`.

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tracing::{debug, info, warn};
use zip::ZipArchive;

use signal_processing::capture::{BlockData, CaptureMetadata, CaptureTransition};
use signal_processing::waveform_index::IndexSampler;
use signal_processing::{
    CaptureIndex, CaptureIndexBuildProgress, CaptureIndexFactory, EdgeQuery, Error, InputPort,
    OutputPort, ProcessNode, ProtocolKind, Result, Sample, SampleBlock, SampleKind, Sender,
    WorkResult,
};

use super::super::capture_archive::zip_error;
use crate::support::capture_index::capture_cache_identity;
use crate::support::dsl_file::{DslChunkedCaptureReader, DslFileCaptureDataSource, parse_header};
use crate::support::get_packed_bit;
const DEFAULT_BLOCK_CACHE_WINDOWS: usize = 2;

type BlockKey = (usize, u64);
type BlockCache = Arc<Mutex<BoundedBlockCache>>;

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
        progress: &mut dyn FnMut(CaptureIndexBuildProgress),
    ) -> Result<Box<dyn CaptureIndex + Send>> {
        let source = DslFileCaptureDataSource::open(&self.path)?;
        IndexSampler::open_data_source_with_progress(source, |value| {
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
/// ## Threading Model
///
/// This is a **self-threading node** (`is_self_threading() = true`). On the first (and only)
/// call to `work()`, it spawns one internal worker thread **per broadcast destination**.
/// The scheduler thread then waits for `should_stop()` to signal completion, rather than
/// calling `work()` repeatedly.
///
/// If a channel is broadcast to multiple receivers, each receiver gets its own independent
/// reading thread. This eliminates head-of-line blocking: slow consumers don't block fast ones.
/// All threads share a single ZipArchive and block cache via `Arc<Mutex<..>>`.
///
/// Example: If channel 0 connects to both `spi_decoder` and `parallel_decoder`, two threads
/// are spawned:
/// - Thread 1: reads channel 0 data → sends to `spi_decoder`
/// - Thread 2: reads channel 0 data → sends to `parallel_decoder`
///
/// If `parallel_decoder` blocks (waiting for enable signal), Thread 2 blocks but Thread 1
/// continues, ensuring `spi_decoder` receives data without interruption.
///
/// # Features
/// - Opens and parses DSLogic capture files (.dsl format)
/// - Per-destination threading eliminates head-of-line blocking
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
    // File access (shared across all channel threads)
    path: PathBuf,
    archive: Arc<Mutex<ZipArchive<File>>>,
    header: CaptureMetadata,
    blocks: BlockCache,

    // Configuration
    num_channels: usize,
    max_samples: Option<u64>,

    // Per-channel thread management
    shutdown: Arc<AtomicBool>,
    threads_completed: Arc<AtomicUsize>,
    thread_handles: Option<Vec<JoinHandle<()>>>,
    threads_spawned: bool,
    num_threads: usize,

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
        let file = File::open(&path)?;
        let mut archive = ZipArchive::new(file).map_err(zip_error)?;
        let header = parse_header(&mut archive)?;

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
            thread_handles: None,
            threads_spawned: false,
            num_threads: 0,
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
        archive: &Arc<Mutex<ZipArchive<File>>>,
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
        // block while this thread was waiting.
        let mut archive_guard = archive.lock().unwrap();
        if let Some(data) = blocks.lock().unwrap().get(key) {
            return Ok(data);
        }
        let block_name = format!("L-{channel}/{block_num}");
        let mut file = archive_guard
            .by_name(&block_name)
            .map_err(|_| Error::InvalidBlock(block_num))?;
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)?;
        let data = BlockData::from(bytes);
        blocks.lock().unwrap().insert(key, data.clone());
        Ok(data)
    }

    /// Worker thread that reads one channel's data and sends to one destination.
    ///
    /// Each thread loads blocks from the shared ZipArchive + cache, walks bits
    /// to detect edges, and sends Samples to its destination. Threads are
    /// fully independent — if a channel is broadcast to multiple destinations,
    /// each destination gets its own thread reading the same channel data.
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
            "[{}] Starting channel reader thread ({} samples, {} blocks)",
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
        // threads do, so a bounded source behaves identically regardless
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

        // First and only call: spawn one thread per connected output destination
        self.threads_spawned = true;

        info!(
            "File source: Spawning per-destination threads for {} samples at {:.1} MHz ({} channels)",
            self.header.total_samples,
            self.header.samplerate_hz / 1_000_000.0,
            self.num_channels
        );

        // Collect all channel-destination pairs to spawn threads for
        // Each destination gets its own independent reader thread. Every
        // channel has exactly one output port now, but that port can
        // carry `Sample` and `SampleBlock` destinations simultaneously
        // (negotiated per connection — see `output_sample_kinds`), so
        // both queries run independently against the same port.
        let mut edge_thread_configs: Vec<(usize, usize, Sender<Sample>, Option<String>)> =
            Vec::new();
        let mut block_thread_groups: HashMap<String, Vec<BlockDestination>> = HashMap::new();

        for channel_idx in 0..self.num_channels {
            let Some(port) = outputs.get(channel_idx) else {
                continue;
            };
            if let Some(senders) = port.split_senders::<Sample>() {
                for (dest_idx, sender) in senders.into_iter().enumerate() {
                    let destination = sender.destination_label().map(str::to_owned);
                    edge_thread_configs.push((channel_idx, dest_idx, sender, destination));
                }
            }
            if let Some(senders) = port.split_senders::<SampleBlock>() {
                for (dest_idx, sender) in senders.into_iter().enumerate() {
                    let destination = sender.destination_label().map(str::to_owned);
                    let group =
                        block_destination_group(channel_idx, dest_idx, destination.as_deref());
                    block_thread_groups
                        .entry(group)
                        .or_default()
                        .push(BlockDestination {
                            channel: channel_idx,
                            sender,
                        });
                }
            }
        }

        let mut handles = Vec::new();

        // Spawn edge reader threads
        for (channel_idx, dest_idx, sender, destination) in edge_thread_configs.into_iter() {
            let archive = Arc::clone(&self.archive);
            let blocks = Arc::clone(&self.blocks);
            let header = self.header.clone();
            let max_samples = self.max_samples;
            let shutdown = Arc::clone(&self.shutdown);
            let completed = Arc::clone(&self.threads_completed);

            let handle = std::thread::Builder::new()
                .name(format!("dsl_ch{}_edge_dest{}", channel_idx, dest_idx))
                .spawn(move || {
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
                })
                .expect("Failed to spawn DslFileSource edge reader thread");

            handles.push(handle);
        }

        // Each destination node receives all of its raw channels from one
        // block-major worker. Different destinations remain independent.
        for (group_label, mut destinations) in block_thread_groups {
            destinations.sort_by_key(|destination| destination.channel);
            let archive = Arc::clone(&self.archive);
            let blocks = Arc::clone(&self.blocks);
            let indexed_blocks = self.index.lock().unwrap().clone();
            let header = self.header.clone();
            let max_samples = self.max_samples;
            let shutdown = Arc::clone(&self.shutdown);
            let completed = Arc::clone(&self.threads_completed);

            let handle = std::thread::Builder::new()
                .name(format!("dsl_blocks_{group_label}"))
                .spawn(move || {
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
                })
                .expect("Failed to spawn DslFileSource block reader thread");

            handles.push(handle);
        }

        self.num_threads = handles.len();
        self.thread_handles = Some(handles);

        info!(
            "File source: Spawned {} reader workers for {} available channels",
            self.num_threads, self.num_channels
        );

        Ok(0)
    }
}

impl Drop for DslFileSource {
    fn drop(&mut self) {
        // Signal all threads to stop
        self.shutdown.store(true, Ordering::Relaxed);

        // Join all thread handles
        if let Some(handles) = self.thread_handles.take() {
            for handle in handles {
                let _ = handle.join();
            }
        }
    }
}

// ============================================================================
// Per-channel thread function
// ============================================================================

/// Configuration for a per-channel reader thread
struct ChannelReaderConfig {
    archive: Arc<Mutex<ZipArchive<File>>>,
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
    archive: Arc<Mutex<ZipArchive<File>>>,
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
    use signal_processing::ProcessNode;

    use super::*;
    use crate::support::dsl_file::DslCaptureReader;
    use crate::support::{get_packed_bit, parse_sample_rate};

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
    fn test_capture_reader_wipneus5_window_if_present() {
        let path = Path::new("_captures/wipneus5.dsl");
        if !path.exists() {
            return;
        }

        let mut reader = DslCaptureReader::open(path)
            .expect("wipneus5.dsl should open with the windowed reader")
            .with_max_cached_blocks(4);
        assert!(reader.header().total_samples > 0);
        assert!(reader.header().total_probes > 0);

        let channel_count = reader.header().total_probes.min(4);
        let channels: Vec<usize> = (0..channel_count).collect();
        let window = reader
            .sampled_window(&channels, 0, 100_000, 800)
            .expect("small wipneus5.dsl viewport should read");

        assert_eq!(window.channels.len(), channel_count);
        assert!(window.sample_step > 0);
    }

    #[test]
    fn test_dsl_channel_edge_index_matches_ground_truth() {
        let path = Path::new("_captures/wipneus5.dsl");
        if !path.exists() {
            return;
        }

        let source = DslFileSource::new(path).expect("wipneus5.dsl should open");
        let edge_query = source
            .edge_query(0, &[])
            .expect("DslFileSource should provide an EdgeQuery for channel 0");

        // Ground truth: exact transitions over a bounded prefix, computed
        // directly against the index (bypassing the galloping wrapper) so
        // this validates next_edge's search logic against real data shape,
        // not just the index itself.
        let ground_truth_end = 2_000_000u64.min(edge_query.total_samples());
        let mut sampler = open_dsl_chunked_capture(path).expect("sampler should open");
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
        let path = Path::new("_captures/wipneus5.dsl");
        if !path.exists() {
            return;
        }
        let source = DslFileSource::new(path).expect("wipneus5.dsl should open");
        let edge_query = source.edge_query(0, &[]).expect("edge query available");
        let limit = 2_000_000u64.min(edge_query.total_samples());

        let Some(first) = edge_query.next_edge(0, limit).unwrap() else {
            return; // channel 0 has no transitions in this prefix; nothing to check
        };

        let same = edge_query
            .next_edge_with_value(0, first.value, limit)
            .unwrap()
            .expect("the first transition itself satisfies its own value");
        assert_eq!(same, first);

        // Edges alternate, so the opposite value's first occurrence (if any
        // before `limit`) is strictly after `first`.
        if let Some(other) = edge_query
            .next_edge_with_value(0, !first.value, limit)
            .unwrap()
        {
            assert_ne!(other.value, first.value);
            assert!(other.sample > first.sample);
        }
    }

    #[test]
    fn test_dsl_channel_edge_index_end_of_file() {
        let path = Path::new("_captures/wipneus5.dsl");
        if !path.exists() {
            return;
        }
        let source = DslFileSource::new(path).expect("wipneus5.dsl should open");
        let edge_query = source.edge_query(0, &[]).expect("edge query available");
        let total = edge_query.total_samples();

        assert_eq!(edge_query.next_edge(total - 1, total).unwrap(), None);
        assert_eq!(edge_query.next_edge(total, total).unwrap(), None);
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_new_valid() {
        // Test with actual scan.dsl file if it exists
        let result = DslFileSource::new("scan.dsl");
        assert!(
            result.is_ok(),
            "Failed to create DslFileSource: {:?}",
            result.err()
        );

        if let Ok(source) = result {
            assert_eq!(source.num_channels(), source.header().total_probes);
            assert_eq!(source.num_inputs(), 0); // Source node
            assert_eq!(source.num_outputs(), source.header().total_probes);
            assert_eq!(source.name(), "dsl_file_source");

            // Check header parsing
            let header = source.header();
            assert!(header.total_probes > 0);
            assert!(header.total_samples > 0);
            assert!(header.samplerate_hz > 0.0);
            assert!(header.sample_period > 0.0);
        }
    }

    #[test]
    fn test_dsl_file_source_invalid_file() {
        let result = DslFileSource::new("nonexistent.dsl");
        assert!(result.is_err());
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_builder_methods() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            let source = source.with_name("custom_source");

            assert_eq!(source.name(), "custom_source");
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_getters() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            assert!(source.total_probes() > 0);
            assert!(source.total_samples() > 0);
            assert!(source.samplerate_hz() > 0.0);
            assert!(source.sample_period() > 0.0);
            assert!(source.capture_duration() > 0.0);

            // Verify relationships
            let expected_duration = source.total_samples() as f64 * source.sample_period();
            assert!((source.capture_duration() - expected_duration).abs() < 0.0001);
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_worknode_methods() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Should not be stopped initially (no threads spawned yet)
            assert!(!source.should_stop());

            // After marking spawned with 0 threads completed, still shouldn't stop
            // (threads_spawned is false initially)
            assert!(!source.threads_spawned);
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_read_bit_valid() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Read first bit from first channel
            let bit_result = source.read_bit(0, 0);
            assert!(
                bit_result.is_ok(),
                "Failed to read bit: {:?}",
                bit_result.err()
            );

            // Read from another channel
            let bit_result = source.read_bit(5, 100);
            assert!(bit_result.is_ok());
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_read_bit_invalid_channel() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Try to read from channel beyond total_probes
            let bit_result = source.read_bit(99, 0);
            assert!(bit_result.is_err());

            if let Err(e) = bit_result {
                match e {
                    Error::InvalidProbe(_) => {}
                    _ => panic!("Expected InvalidProbe error, got {:?}", e),
                }
            }
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_read_bit_invalid_position() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Try to read beyond total_samples
            let bit_result = source.read_bit(0, u64::MAX);
            assert!(bit_result.is_err());

            if let Err(e) = bit_result {
                match e {
                    Error::OutOfBounds(_) => {}
                    _ => panic!("Expected OutOfBounds error, got {:?}", e),
                }
            }
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_header_fields() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            let header = source.header();

            // Verify header fields are populated
            assert!(header.total_probes >= 8);
            assert!(header.total_samples > 0);
            assert!(header.total_blocks > 0);
            assert!(header.samples_per_block > 0);
            assert!(!header.samplerate.is_empty());
            assert!(header.samplerate_hz > 0.0);
            assert!(header.sample_period > 0.0);
            assert!(header.probe_names.len() == header.total_probes);

            // Verify sample rate calculation
            let expected_period = 1.0 / header.samplerate_hz;
            assert!((header.sample_period - expected_period).abs() < 1e-10);

            // Verify samples per block is the actual block size (typically 2^24 = 16777216)
            // This should be larger than the average (total_samples / total_blocks)
            // because the last block is typically shorter
            let average_per_block = header.total_samples / header.total_blocks;
            assert!(header.samples_per_block >= average_per_block);
            // Verify it's a reasonable block size (power of 2 for standard DSL format)
            assert_eq!(header.samples_per_block, 16777216); // 2^24 for scan.dsl
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_block_caching() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Read same bit twice - second read should use cache
            let bit1 = source.read_bit(0, 0);
            let bit2 = source.read_bit(0, 0);

            assert!(bit1.is_ok());
            assert!(bit2.is_ok());
            assert_eq!(bit1.unwrap(), bit2.unwrap());

            // Cache should have entry
            let cache = source.blocks.lock().unwrap();
            assert!(!cache.is_empty(), "Cache should not be empty after reads");
        }
    }

    #[test]
    #[ignore = "requires the developer-local scan.dsl fixture"]
    fn test_dsl_file_source_multiple_channels() {
        let result = DslFileSource::new("scan.dsl");
        assert!(result.is_ok());

        if let Ok(source) = result {
            // Read same position from multiple channels
            let mut channel_values = Vec::new();
            for ch in 0..8 {
                let bit_result = source.read_bit(ch, 1000);
                assert!(
                    bit_result.is_ok(),
                    "Failed to read channel {}: {:?}",
                    ch,
                    bit_result.err()
                );
                channel_values.push(bit_result.unwrap());
            }

            // Should be able to read from all channels
            assert_eq!(channel_values.len(), 8);
        }
    }
}
