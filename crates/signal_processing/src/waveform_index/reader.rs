use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::builder::IndexBuilder;
use super::exact::exact_window_sample_limit;
use super::query::{GroupSummary, SummaryGrid, sample_summary_channel};
use super::resolution::select_summary_resolution;
use super::storage::{IndexReader, IndexWriter, LevelsView};
use super::types::{
    CaptureIndexProgress, SAMPLES_PER_L1_BIT, SAMPLES_PER_L2_BIT, SAMPLES_PER_L3_BIT, bit,
};
use crate::capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureIndex, CaptureIndexBuildProfile,
    CaptureIndexBuildProgress, CaptureIndexOpenStep, CaptureIndexOpenTask, CaptureMetadata,
    CaptureSampledChannel, CaptureSampledWindow, CaptureTransition, packed_bit,
};
use crate::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange, Error, InlineWorkExecutor,
    MemoryArtifactRepository, RepositoryError, Result, SourceIdentity, WorkExecutor,
    read_artifact_region,
};

const RAW_BLOCK_CACHE_CAPACITY: usize = 16;

#[derive(Default)]
struct RawBlockCache {
    entries: HashMap<(usize, u64), BlockData>,
    recency: VecDeque<(usize, u64)>,
}

impl RawBlockCache {
    fn get(&mut self, key: (usize, u64)) -> Option<BlockData> {
        let value = self.entries.get(&key)?.clone();
        self.touch(key);
        Some(value)
    }

    fn insert(&mut self, key: (usize, u64), value: BlockData) {
        self.entries.insert(key, value);
        self.touch(key);
        while self.entries.len() > RAW_BLOCK_CACHE_CAPACITY {
            if let Some(oldest) = self.recency.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }

    fn touch(&mut self, key: (usize, u64)) {
        if self
            .recency
            .back()
            .is_some_and(|candidate| *candidate == key)
        {
            return;
        }
        self.recency.retain(|candidate| *candidate != key);
        self.recency.push_back(key);
    }
}

/// Windowed sampler for indexed capture data.
///
/// Handles index construction/loading and samples visible windows from an
/// an internal index reader, falling back to a raw reader for deep zoom levels.
pub struct IndexSampler<R: BlockCaptureSource> {
    display_name: String,
    storage: IndexReader,
    raw_reader: R,
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    raw_block_cache: RawBlockCache,
    build_profile: Option<CaptureIndexBuildProfile>,
}

impl<R> IndexSampler<R>
where
    R: BlockCaptureSource,
{
    fn new(
        display_name: String,
        storage: IndexReader,
        raw_reader: R,
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        build_profile: Option<CaptureIndexBuildProfile>,
    ) -> Self {
        Self {
            display_name,
            storage,
            raw_reader,
            repository,
            identity,
            raw_block_cache: RawBlockCache::default(),
            build_profile,
        }
    }

    /// Opens a data source, building or loading its waveform index synchronously.
    ///
    /// # Parameters
    /// - `data_source`: Capture source whose raw blocks and index artifacts are opened.
    pub fn open_data_source<S>(data_source: S) -> Result<Self>
    where
        S: CaptureDataSource<Reader = R>,
    {
        Self::open_data_source_with_executor_and_progress(
            data_source,
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(InlineWorkExecutor),
            |_| true,
        )
    }

    /// Opens a data source while reporting synchronous index-build progress.
    ///
    /// Returning `false` from `progress` cancels construction.
    ///
    /// # Parameters
    ///
    /// - `data_source`: Capture source whose raw blocks and index artifacts are opened.
    /// - `progress`: Callback invoked as root-block summaries complete.
    pub fn open_data_source_with_progress<S, C>(data_source: S, progress: C) -> Result<Self>
    where
        S: CaptureDataSource<Reader = R>,
        C: FnMut(CaptureIndexProgress) -> bool,
    {
        Self::open_data_source_with_executor_and_progress(
            data_source,
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(InlineWorkExecutor),
            progress,
        )
    }

    /// Starts a sequential index build that yields after every channel block.
    ///
    /// This is intended for host workers that must return to their event loop
    /// between bounded units of file and index work. The published index is
    /// byte-for-byte identical to the synchronous builder's output.
    pub fn begin_open_data_source<S>(
        data_source: S,
        repository: Arc<dyn ArtifactRepository>,
    ) -> Result<Box<dyn CaptureIndexOpenTask>>
    where
        S: CaptureDataSource<Reader = R>,
    {
        Ok(Box::new(IncrementalIndexSamplerTask::new(
            data_source,
            repository,
        )?))
    }

    /// Opens a data source using a host executor for index construction.
    ///
    /// # Parameters
    ///
    /// - `data_source`: Capture source whose raw blocks and index artifacts are opened.
    /// - `repository`: Artifact repository that holds persistent index pages.
    /// - `work_executor`: Host capability used for construction work.
    /// - `progress`: Callback invoked as root-block summaries complete.
    pub fn open_data_source_with_executor_and_progress<S, C>(
        data_source: S,
        repository: Arc<dyn ArtifactRepository>,
        work_executor: Arc<dyn WorkExecutor>,
        progress: C,
    ) -> Result<Self>
    where
        S: CaptureDataSource<Reader = R>,
        C: FnMut(CaptureIndexProgress) -> bool,
    {
        let header = data_source.metadata().clone();
        let fingerprint = data_source.fingerprint();
        let identity = data_source
            .index_identity()
            .ok_or_else(|| Error::ParseError("capture source is not indexable".to_string()))?;

        let build_profile = if !IndexReader::is_valid(
            repository.as_ref(),
            identity,
            &header,
            fingerprint.revision,
        )? {
            Some(
                IndexBuilder::new(
                    &data_source,
                    Arc::clone(&repository),
                    identity,
                    &header,
                    fingerprint.revision,
                )
                .build(work_executor, progress)?,
            )
        } else {
            None
        };

        let storage = IndexReader::open(
            Arc::clone(&repository),
            identity,
            header,
            fingerprint.revision,
        )?;
        let display_name = data_source.display_name();
        let raw_reader = data_source.open_reader()?;
        Ok(Self::new(
            display_name,
            storage,
            raw_reader,
            repository,
            identity,
            build_profile,
        ))
    }

    /// Reopens a complete published index without constructing a missing one.
    ///
    /// Runtime capability discovery uses this path because it must remain
    /// bounded and non-blocking. Index construction belongs to source
    /// preparation, where the host can schedule it away from the caller.
    ///
    /// # Parameters
    /// - `data_source`: Capture source whose identity selects an existing index.
    /// - `repository`: Artifact repository that may contain that index.
    pub fn open_existing_data_source<S>(
        data_source: S,
        repository: Arc<dyn ArtifactRepository>,
    ) -> Result<Option<Self>>
    where
        S: CaptureDataSource<Reader = R>,
    {
        let header = data_source.metadata().clone();
        let fingerprint = data_source.fingerprint();
        let identity = data_source
            .index_identity()
            .ok_or_else(|| Error::ParseError("capture source is not indexable".to_string()))?;
        if !IndexReader::is_valid(repository.as_ref(), identity, &header, fingerprint.revision)? {
            return Ok(None);
        }

        let storage = IndexReader::open(
            Arc::clone(&repository),
            identity,
            header,
            fingerprint.revision,
        )?;
        let display_name = data_source.display_name();
        let raw_reader = data_source.open_reader()?;
        Ok(Some(Self::new(
            display_name,
            storage,
            raw_reader,
            repository,
            identity,
            None,
        )))
    }

    /// Returns a display name for this indexed capture.
    pub fn display_name(&self) -> String {
        self.display_name.clone()
    }

    /// Returns the source identity that keys this index's artifacts.
    pub fn index_identity(&self) -> SourceIdentity {
        self.storage.identity()
    }

    /// Returns immutable capture metadata used by every query.
    pub fn header(&self) -> &CaptureMetadata {
        self.storage.header()
    }

    /// Returns measurements from the cold build that produced this sampler.
    /// Reopened indexes return `None` because no build ran in this process.
    pub fn build_profile(&self) -> Option<CaptureIndexBuildProfile> {
        self.build_profile
    }

    /// Returns capture duration in microseconds.
    pub fn capture_duration_us(&self) -> f64 {
        self.header().total_samples as f64 * 1_000_000.0 / self.header().samplerate_hz
    }

    /// Fraction of 64-sample groups containing one or more transitions.
    /// Reads only the waveform-index artifacts; raw capture blocks and the
    /// raw-block cache are never touched.
    pub fn activity_ratio_hint(&self, channel: usize, limit: u64) -> Result<f64> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        let limit = limit.min(self.header().total_samples);
        if limit == 0 {
            return Ok(0.0);
        }

        let samples_per_block = self.header().samples_per_block;
        let blocks = limit.div_ceil(samples_per_block);
        let mut active_groups = 0u64;
        let mut total_groups = 0u64;
        for block in 0..blocks {
            let block_start = block * samples_per_block;
            let valid_samples = (limit - block_start).min(samples_per_block);
            let groups = valid_samples.div_ceil(SAMPLES_PER_L1_BIT) as usize;
            total_groups += groups as u64;

            let root = self.storage.load_root_summary(channel, block as usize)?;
            if !root.toggle {
                continue;
            }
            let leaf = self.storage.load_leaf(channel, block as usize)?;
            let Some(levels) = leaf.levels else {
                continue;
            };
            let full_words = groups / u64::BITS as usize;
            active_groups += levels.l1_toggle[..full_words]
                .iter()
                .map(|word| u64::from(word.count_ones()))
                .sum::<u64>();
            let remainder = groups % u64::BITS as usize;
            if remainder > 0 {
                let mask = (1u64 << remainder) - 1;
                active_groups += u64::from((levels.l1_toggle[full_words] & mask).count_ones());
            }
        }
        Ok(active_groups as f64 / total_groups.max(1) as f64)
    }

    /// Estimated high-level occupancy from the final value of each 64-sample
    /// index group. Reads only waveform-index artifacts and never touches raw
    /// capture blocks.
    ///
    /// # Parameters
    /// - `channel`: Zero-based capture channel to summarize.
    /// - `limit`: Exclusive sample bound for the estimate.
    pub fn high_level_ratio_hint(&self, channel: usize, limit: u64) -> Result<f64> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        let limit = limit.min(self.header().total_samples);
        if limit == 0 {
            return Ok(0.0);
        }

        let samples_per_block = self.header().samples_per_block;
        let blocks = limit.div_ceil(samples_per_block);
        let mut high_groups = 0u64;
        let mut total_groups = 0u64;
        for block in 0..blocks {
            let block_start = block * samples_per_block;
            let valid_samples = (limit - block_start).min(samples_per_block);
            let groups = valid_samples.div_ceil(SAMPLES_PER_L1_BIT) as usize;
            total_groups += groups as u64;

            let root = self.storage.load_root_summary(channel, block as usize)?;
            if !root.toggle {
                if root.last {
                    high_groups += groups as u64;
                }
                continue;
            }
            let leaf = self.storage.load_leaf(channel, block as usize)?;
            let Some(levels) = leaf.levels else {
                if leaf.last {
                    high_groups += groups as u64;
                }
                continue;
            };
            let full_words = groups / u64::BITS as usize;
            high_groups += levels.l1_last[..full_words]
                .iter()
                .map(|word| u64::from(word.count_ones()))
                .sum::<u64>();
            let remainder = groups % u64::BITS as usize;
            if remainder > 0 {
                let mask = (1u64 << remainder) - 1;
                high_groups += u64::from((levels.l1_last[full_words] & mask).count_ones());
            }
        }
        Ok(high_groups as f64 / total_groups.max(1) as f64)
    }

    /// Value of `channel` at `position`. O(1) after the containing block is
    /// cached (archive capture store, or the raw reader's own LRU).
    pub fn value_at(&mut self, channel: usize, position: u64) -> Result<bool> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        if position >= self.header().total_samples {
            return Err(Error::OutOfBounds(position));
        }
        let samples_per_block = self.header().samples_per_block;
        let block = position / samples_per_block;
        let data = self.cached_packed_block(channel, block)?;
        Ok(packed_bit(&data, (position % samples_per_block) as usize))
    }

    /// First transition strictly after `position` and before `limit`.
    ///
    /// The display index is descended from its block summary through L3,
    /// L2, and L1. Only the final 64-sample candidate group touches the raw
    /// packed data. This keeps long constant ranges index-only and avoids
    /// constructing a complete sampled-window transition vector when the
    /// caller needs just one edge.
    pub fn next_transition(
        &mut self,
        channel: usize,
        position: u64,
        limit: u64,
    ) -> Result<Option<CaptureTransition>> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }

        let limit = limit.min(self.header().total_samples);
        let Some(mut search) = position.checked_add(1) else {
            return Ok(None);
        };
        if search >= limit {
            return Ok(None);
        }

        let samples_per_block = self.header().samples_per_block;
        while search < limit {
            let block = search / samples_per_block;
            let block_start = block * samples_per_block;
            let block_limit = block_start.saturating_add(samples_per_block).min(limit);
            let local_limit = block_limit - block_start;

            let root = self.storage.load_root_summary(channel, block as usize)?;
            if !root.toggle {
                search = block_limit;
                continue;
            }

            let local_search = search - block_start;
            let candidate = {
                let leaf = self.storage.load_leaf(channel, block as usize)?;
                leaf.levels
                    .as_ref()
                    .and_then(|levels| next_indexed_l1_group(levels, local_search, local_limit))
            };
            let Some(l1_group) = candidate else {
                search = block_limit;
                continue;
            };

            let group_start = l1_group as u64 * SAMPLES_PER_L1_BIT;
            let scan_start = local_search.max(group_start);
            let scan_end = local_limit.min(group_start + SAMPLES_PER_L1_BIT);
            if let Some(transition) =
                self.next_raw_transition(channel, block, scan_start, scan_end)?
            {
                return Ok(Some(transition));
            }

            // The index can mark a group because of a transition at its
            // first sample that is at/before `position`. Once the exact scan
            // proves there is no later edge in the group, skip it entirely.
            search = block_start + scan_end;
        }

        Ok(None)
    }

    /// Appends up to `max_transitions` exact transitions after `position` and
    /// before `limit`, descending the index once per active 64-sample group.
    ///
    /// # Parameters
    /// - `channel`: Zero-based capture channel to inspect.
    /// - `position`: First sample position to search after.
    /// - `limit`: Exclusive sample bound for returned transitions.
    /// - `max_transitions`: Maximum transitions appended to `output`.
    /// - `output`: Destination vector for ordered transitions.
    pub fn next_transitions(
        &mut self,
        channel: usize,
        position: u64,
        limit: u64,
        max_transitions: usize,
        output: &mut Vec<CaptureTransition>,
    ) -> Result<()> {
        output.clear();
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        if max_transitions == 0 {
            return Ok(());
        }

        let limit = limit.min(self.header().total_samples);
        let Some(mut search) = position.checked_add(1) else {
            return Ok(());
        };
        if search >= limit {
            return Ok(());
        }

        output.reserve(max_transitions.min(65_536));
        let samples_per_block = self.header().samples_per_block;
        while search < limit && output.len() < max_transitions {
            let block = search / samples_per_block;
            let block_start = block * samples_per_block;
            let block_limit = block_start.saturating_add(samples_per_block).min(limit);
            let local_limit = block_limit - block_start;
            let root = self.storage.load_root_summary(channel, block as usize)?;
            if !root.toggle {
                search = block_limit;
                continue;
            }

            // Acquire one packed block view and one leaf view for every
            // contiguous search through this block. Repository byte regions
            // preserve their native mmap or owned-memory backing here.
            let data = self.cached_packed_block(channel, block)?;
            let leaf = self.storage.load_leaf(channel, block as usize)?;
            let Some(levels) = leaf.levels.as_ref() else {
                search = block_limit;
                continue;
            };
            let previous_block_last = if block > 0 {
                Some(
                    self.storage
                        .load_root_summary(channel, block as usize - 1)?
                        .last,
                )
            } else {
                None
            };

            let mut local_search = search - block_start;
            while local_search < local_limit && output.len() < max_transitions {
                let Some(l1_group) = next_indexed_l1_group(levels, local_search, local_limit)
                else {
                    break;
                };
                let group_start = l1_group as u64 * SAMPLES_PER_L1_BIT;
                let scan_start = local_search.max(group_start);
                let scan_end = local_limit.min(group_start + SAMPLES_PER_L1_BIT);
                append_raw_transitions(
                    &data,
                    block_start,
                    scan_start,
                    scan_end,
                    previous_block_last,
                    max_transitions,
                    output,
                );
                local_search = scan_end;
            }
            search = block_start + local_search;
            if local_search >= local_limit
                || next_indexed_l1_group(levels, local_search, local_limit).is_none()
            {
                search = block_limit;
            }
        }
        Ok(())
    }

    /// Reads a sorted batch of positions with one packed block acquisition
    /// per block instead of one acquisition per sample.
    ///
    /// # Parameters
    /// - `channel`: Zero-based capture channel to read.
    /// - `positions`: Sorted absolute sample positions to query.
    /// - `output`: Destination vector that receives values in input order.
    pub fn values_at(
        &mut self,
        channel: usize,
        positions: &[u64],
        output: &mut Vec<bool>,
    ) -> Result<()> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }
        output.clear();
        output.reserve(positions.len());

        let samples_per_block = self.header().samples_per_block;
        let mut cursor = 0;
        while cursor < positions.len() {
            let position = positions[cursor];
            if position >= self.header().total_samples {
                return Err(Error::OutOfBounds(position));
            }
            let block = position / samples_per_block;
            let data = self.cached_packed_block(channel, block)?;
            while cursor < positions.len() {
                let position = positions[cursor];
                if position >= self.header().total_samples {
                    return Err(Error::OutOfBounds(position));
                }
                if position / samples_per_block != block {
                    break;
                }
                output.push(packed_bit(&data, (position % samples_per_block) as usize));
                cursor += 1;
            }
        }
        Ok(())
    }

    fn next_raw_transition(
        &mut self,
        channel: usize,
        block: u64,
        local_start: u64,
        local_end: u64,
    ) -> Result<Option<CaptureTransition>> {
        if local_start >= local_end {
            return Ok(None);
        }

        let samples_per_block = self.header().samples_per_block;
        let block_start = block * samples_per_block;
        let data = self.cached_packed_block(channel, block)?;
        let word_index = local_start as usize / 64;
        let word_start = word_index * 64;
        let word = load_le_word(&data, word_index);
        let entering = if word_start > 0 {
            packed_bit(&data, word_start - 1)
        } else if block > 0 {
            self.storage
                .load_root_summary(channel, block as usize - 1)?
                .last
        } else {
            // Sample zero has no predecessor and therefore is not itself a
            // transition. Treat its own value as the entering level.
            word & 1 != 0
        };

        let lo = local_start as usize - word_start;
        let hi = local_end as usize - word_start;
        let mut toggles = word ^ ((word << 1) | entering as u64);
        toggles &= range_mask(lo, hi);
        let Some(bit_index) = nonzero_trailing_bit(toggles) else {
            return Ok(None);
        };

        Ok(Some(CaptureTransition {
            sample: block_start + (word_start + bit_index) as u64,
            value: bit(word, bit_index),
        }))
    }

    /// Samples a multi-channel window at a resolution suitable for rendering.
    ///
    /// # Parameters
    /// - `channels`: Zero-based capture channels to sample.
    /// - `start_sample`: Inclusive first sample in the requested window.
    /// - `end_sample`: Exclusive end of the requested window.
    /// - `target_points`: Rendering resolution that selects index coarseness.
    pub fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        let total_samples = self.header().total_samples;
        let start_sample = start_sample.min(total_samples.saturating_sub(1));
        let end_sample = end_sample.clamp(start_sample + 1, total_samples);
        let samples = end_sample - start_sample;
        let target_points = target_points.max(1);

        if samples <= exact_window_sample_limit(target_points) {
            return self.exact_sampled_window(channels, start_sample, end_sample);
        }

        let group_samples = select_summary_resolution(
            samples,
            target_points,
            [
                SAMPLES_PER_L1_BIT,
                SAMPLES_PER_L2_BIT,
                SAMPLES_PER_L3_BIT,
                self.header().samples_per_block,
            ],
        )
        .expect("waveform indexes always provide an L1 summary");

        let mut sampled_channels = Vec::with_capacity(channels.len());
        for &channel in channels {
            sampled_channels.push(self.sample_indexed_channel(
                channel,
                start_sample,
                end_sample,
                target_points,
                group_samples,
            )?);
        }

        Ok(CaptureSampledWindow {
            start_sample,
            end_sample,
            sample_step: group_samples,
            channels: sampled_channels,
        })
    }

    fn exact_sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
    ) -> Result<CaptureSampledWindow> {
        let mut sampled_channels = Vec::with_capacity(channels.len());
        for &channel in channels {
            sampled_channels.push(self.exact_sampled_channel(channel, start_sample, end_sample)?);
        }

        Ok(CaptureSampledWindow {
            start_sample,
            end_sample,
            sample_step: 1,
            channels: sampled_channels,
        })
    }

    fn exact_sampled_channel(
        &mut self,
        channel: usize,
        start_sample: u64,
        end_sample: u64,
    ) -> Result<CaptureSampledChannel> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }

        let name = self
            .header()
            .probe_names
            .get(channel)
            .cloned()
            .unwrap_or_else(|| format!("Probe{}", channel));
        let samples_per_block = self.header().samples_per_block;
        let first_block = start_sample / samples_per_block;
        let last_block = (end_sample - 1) / samples_per_block;
        let mut current = {
            let data = self.cached_packed_block(channel, first_block)?;
            packed_bit(&data, (start_sample % samples_per_block) as usize)
        };
        let initial = current;
        let mut transitions = Vec::new();

        for block in first_block..=last_block {
            let data = self.cached_packed_block(channel, block)?;
            let block_start = block * samples_per_block;
            let block_end = block_start
                .saturating_add(samples_per_block)
                .min(end_sample);
            // Transitions are reported from the second window sample onwards.
            let scan_start = block_start.max(start_sample + 1);
            if scan_start >= block_end {
                continue;
            }

            let lo_local = (scan_start - block_start) as usize;
            let hi_local = (block_end - block_start) as usize;
            let first_word = lo_local / 64;
            let last_word = (hi_local - 1) / 64;
            for word_index in first_word..=last_word {
                let word = load_le_word(&data, word_index);
                let lo = if word_index == first_word {
                    lo_local % 64
                } else {
                    0
                };
                let hi = if word_index == last_word {
                    hi_local - word_index * 64
                } else {
                    64
                };

                // Bit i marks a change between sample i and sample i-1; the
                // shifted-in bit 0 compares against `current`, the value of
                // the last sample processed before this word.
                let mut toggles = word ^ ((word << 1) | current as u64);
                toggles &= range_mask(lo, hi);

                while toggles != 0 {
                    let bit_index = toggles.trailing_zeros() as usize;
                    toggles &= toggles - 1;
                    let value = (word >> bit_index) & 1 != 0;
                    transitions.push(CaptureTransition {
                        sample: block_start + (word_index * 64 + bit_index) as u64,
                        value,
                    });
                }
                current = (word >> (hi - 1)) & 1 != 0;
            }
        }

        Ok(CaptureSampledChannel {
            channel,
            name,
            initial,
            transitions,
            waveform: Vec::new(),
        })
    }

    /// Packed block bytes, preferring the sparse archive store over the (usually
    /// compressed) capture source; freshly decompressed blocks are stored in
    /// the cache for later zero-copy reads.
    fn cached_packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData> {
        let key = (channel, block);
        if let Some(data) = self.raw_block_cache.get(key) {
            return Ok(data);
        }
        let data = if let Some(data) = self.cached_raw_block(channel, block)? {
            data
        } else {
            let data = self.raw_reader.read_packed_block(channel, block)?;
            self.publish_raw_block(channel, block, &data)?;
            data
        };
        self.raw_block_cache.insert(key, data.clone());
        Ok(data)
    }

    /// Returns one packed capture block for an external streaming source.
    ///
    /// Existing archive-store entries are reused, but a miss is read directly
    /// from the capture source without populating the cache. Sequential graph
    /// processing may visit the complete capture, whereas the archive store is
    /// intentionally sparse and reserved for regions inspected through
    /// random-access queries.
    ///
    /// # Parameters
    /// - `channel`: Input consumed by this operation.
    /// - `block`: Input consumed by this operation.
    pub fn packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData> {
        if let Some(data) = self.cached_raw_block(channel, block)? {
            return Ok(data);
        }
        self.raw_reader.read_packed_block(channel, block)
    }

    fn cached_raw_block(&self, channel: usize, block: u64) -> Result<Option<BlockData>> {
        let key = raw_block_key(self.identity, channel, block)?;
        let Some(mut reader) = self.repository.open(&key).map_err(repository_error)? else {
            return Ok(None);
        };
        let range = ByteRange::new(0, reader.len().map_err(repository_error)?)
            .map_err(|error| Error::ParseError(error.to_string()))?;
        let backing = read_artifact_region(reader.as_mut(), range).map_err(repository_error)?;
        let region = crate::ByteRegion::new(backing, range)
            .map_err(|error| Error::ParseError(error.to_string()))?;
        Ok(Some(BlockData::from_region(region)))
    }

    fn publish_raw_block(&self, channel: usize, block: u64, data: &[u8]) -> Result<()> {
        let mut writer = self
            .repository
            .begin_write(raw_block_key(self.identity, channel, block)?)
            .map_err(repository_error)?;
        writer.write_at(0, data).map_err(repository_error)?;
        writer
            .truncate(data.len() as u64)
            .map_err(repository_error)?;
        // This exact block is an opportunistic, rebuildable cache. Atomic
        // publication matters to concurrent readers; forcing durable media
        // synchronization for every random-access miss does not.
        writer.publish().map_err(repository_error)
    }

    fn sample_indexed_channel(
        &mut self,
        channel: usize,
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
        group_samples: u64,
    ) -> Result<CaptureSampledChannel> {
        if channel >= self.header().total_probes {
            return Err(Error::InvalidProbe(channel));
        }

        let name = self
            .header()
            .probe_names
            .get(channel)
            .cloned()
            .unwrap_or_else(|| format!("Probe{}", channel));
        let initial = self.indexed_initial_value(channel, start_sample, group_samples)?;
        sample_summary_channel(
            channel,
            name,
            initial,
            SummaryGrid {
                start_sample,
                available_end_sample: end_sample,
                grid_end_sample: end_sample,
                target_points,
            },
            |visible_start, visible_end, previous_value| {
                self.indexed_display_range_summary(
                    channel,
                    visible_start,
                    visible_end,
                    group_samples,
                    previous_value,
                )
            },
        )
    }

    /// Group-aligned value entering `sample`, derived purely from the index.
    /// Keeps the indexed path free of raw-capture reads (which decompress
    /// whole blocks); consistent with the per-pixel summaries, which are
    /// aligned to the same groups.
    fn indexed_initial_value(
        &mut self,
        channel: usize,
        sample: u64,
        group_samples: u64,
    ) -> Result<bool> {
        let block = self.block_for_sample(sample);
        let local = sample % self.header().samples_per_block;
        Ok(self
            .block_local_display_summary(channel, block as usize, local, local + 1, group_samples)?
            .first)
    }

    fn indexed_display_range_summary(
        &mut self,
        channel: usize,
        start_sample: u64,
        end_sample: u64,
        group_samples: u64,
        fallback_first: bool,
    ) -> Result<GroupSummary> {
        let mut first = None;
        let mut last = fallback_first;
        let mut toggle = false;
        let samples_per_block = self.header().samples_per_block;
        let first_block = self.block_for_sample(start_sample);
        let last_block = self.block_for_sample(end_sample.saturating_sub(1));

        for block in first_block..=last_block {
            let block_start = block * samples_per_block;
            let local_start = start_sample.saturating_sub(block_start);
            let local_end = end_sample
                .saturating_sub(block_start)
                .min(samples_per_block);
            if local_end <= local_start {
                continue;
            }

            let summary = self.block_local_display_summary(
                channel,
                block as usize,
                local_start,
                local_end,
                group_samples,
            )?;
            let range_first = *first.get_or_insert(summary.first);
            toggle |= summary.toggle || summary.first != last || range_first != summary.last;
            last = summary.last;
        }

        Ok(GroupSummary {
            first: first.unwrap_or(fallback_first),
            toggle,
            last,
        })
    }

    fn block_local_display_summary(
        &mut self,
        channel: usize,
        block: usize,
        local_start: u64,
        local_end: u64,
        group_samples: u64,
    ) -> Result<GroupSummary> {
        let samples_per_block = self.header().samples_per_block;
        let block_start = block as u64 * samples_per_block;
        let valid_end = self
            .header()
            .total_samples
            .saturating_sub(block_start)
            .min(samples_per_block);

        if local_start == 0 && local_end >= valid_end {
            let entry = self.storage.load_root_summary(channel, block)?;
            return Ok(GroupSummary {
                first: entry.first,
                toggle: entry.toggle,
                last: entry.last,
            });
        }

        if group_samples >= SAMPLES_PER_L3_BIT {
            let entry = self.storage.load_root_summary(channel, block)?;
            // Constant blocks store no level bitmaps; their l3_last/l3_toggle
            // words are zero and must not be interpreted as sample values.
            if !entry.toggle {
                return Ok(GroupSummary {
                    first: entry.first,
                    toggle: false,
                    last: entry.last,
                });
            }
            let first_group = (local_start / SAMPLES_PER_L3_BIT).min(63) as usize;
            let last_group = ((local_end - 1) / SAMPLES_PER_L3_BIT).min(63) as usize;
            let first = if first_group == 0 {
                entry.first
            } else {
                bit(entry.l3_last, first_group - 1)
            };
            let last = bit(entry.l3_last, last_group);
            return Ok(GroupSummary {
                first,
                toggle: bit_range_any(&[entry.l3_toggle], first_group, last_group) || first != last,
                last,
            });
        }

        let leaf = self.storage.load_leaf(channel, block)?;
        let Some(levels) = leaf.levels else {
            return Ok(GroupSummary {
                first: leaf.first,
                toggle: false,
                last: leaf.last,
            });
        };

        if group_samples >= SAMPLES_PER_L2_BIT {
            let first_group = (local_start / SAMPLES_PER_L2_BIT).min(4095) as usize;
            let last_group = ((local_end - 1) / SAMPLES_PER_L2_BIT).min(4095) as usize;
            let first = if first_group == 0 {
                leaf.first
            } else {
                bit(
                    levels.l2_last[(first_group - 1) / 64],
                    (first_group - 1) % 64,
                )
            };
            let last = bit(levels.l2_last[last_group / 64], last_group % 64);
            Ok(GroupSummary {
                first,
                toggle: bit_range_any(&levels.l2_toggle, first_group, last_group) || first != last,
                last,
            })
        } else {
            let first_group = (local_start / SAMPLES_PER_L1_BIT).min(262_143) as usize;
            let last_group = ((local_end - 1) / SAMPLES_PER_L1_BIT).min(262_143) as usize;
            let first = if first_group == 0 {
                leaf.first
            } else {
                bit(
                    levels.l1_last[(first_group - 1) / 64],
                    (first_group - 1) % 64,
                )
            };
            let last = bit(levels.l1_last[last_group / 64], last_group % 64);
            Ok(GroupSummary {
                first,
                toggle: bit_range_any(&levels.l1_toggle, first_group, last_group) || first != last,
                last,
            })
        }
    }

    fn block_for_sample(&self, sample: u64) -> u64 {
        sample / self.header().samples_per_block
    }
}

struct IncrementalIndexSamplerTask<S>
where
    S: CaptureDataSource,
{
    data_source: S,
    reader: Option<S::Reader>,
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    header: CaptureMetadata,
    source_revision: u64,
    writer: Option<IndexWriter>,
    previous_last: Vec<Option<bool>>,
    channel: usize,
    block: u64,
    completed: u64,
    total: u64,
    ready: Option<Box<dyn CaptureIndex + Send>>,
}

impl<S> IncrementalIndexSamplerTask<S>
where
    S: CaptureDataSource,
{
    fn new(data_source: S, repository: Arc<dyn ArtifactRepository>) -> Result<Self> {
        let header = data_source.metadata().clone();
        let fingerprint = data_source.fingerprint();
        let identity = data_source
            .index_identity()
            .ok_or_else(|| Error::ParseError("capture source is not indexable".to_owned()))?;
        let total = u64::try_from(header.total_probes)
            .ok()
            .and_then(|channels| channels.checked_mul(header.total_blocks))
            .ok_or_else(|| Error::ParseError("capture-index job count overflow".to_owned()))?;

        if IndexReader::is_valid(repository.as_ref(), identity, &header, fingerprint.revision)? {
            let storage = IndexReader::open(
                Arc::clone(&repository),
                identity,
                header.clone(),
                fingerprint.revision,
            )?;
            let display_name = data_source.display_name();
            let raw_reader = data_source.open_reader()?;
            let ready = Box::new(IndexSampler::new(
                display_name,
                storage,
                raw_reader,
                Arc::clone(&repository),
                identity,
                None,
            ));
            return Ok(Self {
                data_source,
                reader: None,
                repository,
                identity,
                header,
                source_revision: fingerprint.revision,
                writer: None,
                previous_last: Vec::new(),
                channel: 0,
                block: 0,
                completed: total,
                total,
                ready: Some(ready),
            });
        }

        let reader = data_source.open_reader()?;
        let writer = IndexWriter::create(
            Arc::clone(&repository),
            identity,
            &header,
            fingerprint.revision,
        )?;
        Ok(Self {
            data_source,
            reader: Some(reader),
            repository,
            identity,
            previous_last: vec![None; header.total_probes],
            header,
            source_revision: fingerprint.revision,
            writer: Some(writer),
            channel: 0,
            block: 0,
            completed: 0,
            total,
            ready: None,
        })
    }

    fn finish(&mut self) -> Result<CaptureIndexOpenStep> {
        self.writer
            .take()
            .ok_or_else(|| Error::ParseError("capture-index writer is unavailable".to_owned()))?
            .finish()?;
        let storage = IndexReader::open(
            Arc::clone(&self.repository),
            self.identity,
            self.header.clone(),
            self.source_revision,
        )?;
        let reader = self
            .reader
            .take()
            .ok_or_else(|| Error::ParseError("capture reader is unavailable".to_owned()))?;
        Ok(CaptureIndexOpenStep::Ready(Box::new(IndexSampler::new(
            self.data_source.display_name(),
            storage,
            reader,
            Arc::clone(&self.repository),
            self.identity,
            None,
        ))))
    }
}

impl<S> CaptureIndexOpenTask for IncrementalIndexSamplerTask<S>
where
    S: CaptureDataSource,
{
    fn expected_index_identity(&self) -> Option<SourceIdentity> {
        Some(self.identity)
    }

    fn step(&mut self) -> Result<CaptureIndexOpenStep> {
        if let Some(index) = self.ready.take() {
            return Ok(CaptureIndexOpenStep::Ready(index));
        }
        if self.completed >= self.total {
            return self.finish();
        }

        let data = self
            .reader
            .as_mut()
            .ok_or_else(|| Error::ParseError("capture reader is unavailable".to_owned()))?
            .read_packed_block(self.channel, self.block)?;
        let block_start = self.block.saturating_mul(self.header.samples_per_block);
        let valid_samples = u64::try_from(data.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(8)
            .min(self.header.total_samples.saturating_sub(block_start));
        let mut leaf = IndexBuilder::<S>::build_leaf(&data, valid_samples)?;
        IndexBuilder::<S>::apply_boundary_transition(&mut leaf, self.previous_last[self.channel]);
        self.previous_last[self.channel] = Some(leaf.last);
        let block = usize::try_from(self.block).map_err(|_| {
            Error::ParseError("capture-index block exceeds this address space".to_owned())
        })?;
        self.writer
            .as_mut()
            .ok_or_else(|| Error::ParseError("capture-index writer is unavailable".to_owned()))?
            .write_block(self.channel, block, &leaf)?;

        self.completed = self.completed.saturating_add(1);
        self.block = self.block.saturating_add(1);
        if self.block >= self.header.total_blocks {
            self.block = 0;
            self.channel += 1;
        }
        Ok(CaptureIndexOpenStep::Progress(CaptureIndexBuildProgress {
            completed: self.completed,
            total: self.total,
        }))
    }
}

/// Loads the 64-sample word at `word_index` from LSB-first packed bytes,
/// zero-padding past the end of `data` (callers mask out padded bits).
fn load_le_word(data: &[u8], word_index: usize) -> u64 {
    let byte_start = word_index * 8;
    if let Some(chunk) = data.get(byte_start..byte_start + 8) {
        u64::from_le_bytes(chunk.try_into().expect("chunk is 8 bytes"))
    } else {
        let mut bytes = [0_u8; 8];
        let available = data.len().saturating_sub(byte_start).min(8);
        bytes[..available].copy_from_slice(&data[byte_start..byte_start + available]);
        u64::from_le_bytes(bytes)
    }
}

/// Mask selecting bits `lo..hi` (hi exclusive, hi ≤ 64).
fn range_mask(lo: usize, hi: usize) -> u64 {
    let upper = if hi == 64 {
        u64::MAX
    } else {
        (1_u64 << hi) - 1
    };
    upper & !((1_u64 << lo) - 1)
}

fn raw_block_key(identity: SourceIdentity, channel: usize, block: u64) -> Result<ArtifactKey> {
    let namespace = ArtifactNamespace::new("capture-raw-block-v1").map_err(repository_error)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(namespace.as_str().as_bytes());
    hasher.update(identity.as_bytes());
    hasher.update(&(channel as u64).to_le_bytes());
    hasher.update(&block.to_le_bytes());
    Ok(ArtifactKey::new(
        namespace,
        SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
    ))
}

fn repository_error(error: RepositoryError) -> Error {
    Error::ParseError(error.to_string())
}

fn bit_range_any(words: &[u64], first_bit: usize, last_bit: usize) -> bool {
    if last_bit < first_bit {
        return false;
    }

    let first_word = first_bit / 64;
    let last_word = last_bit / 64;
    for word_index in first_word..=last_word {
        let Some(mut word) = words.get(word_index).copied() else {
            break;
        };
        if word_index == first_word {
            word &= u64::MAX << (first_bit % 64);
        }
        if word_index == last_word {
            let end_bit = last_bit % 64;
            let mask = if end_bit == 63 {
                u64::MAX
            } else {
                (1_u64 << (end_bit + 1)) - 1
            };
            word &= mask;
        }
        if word != 0 {
            return true;
        }
    }
    false
}

/// Finds the first active 64-sample L1 group in `start..end` by descending
/// the L3 -> L2 -> L1 toggle summaries. `start` and `end` are block-local
/// sample positions and `end` is exclusive.
fn next_indexed_l1_group(levels: &LevelsView, start: u64, end: u64) -> Option<usize> {
    if start >= end {
        return None;
    }

    let first_l3 = (start / SAMPLES_PER_L3_BIT) as usize;
    let last_l3 = ((end - 1) / SAMPLES_PER_L3_BIT).min(63) as usize;
    for l3 in first_l3..=last_l3 {
        if !bit(levels.l3_toggle, l3) {
            continue;
        }

        let l3_first_l2 = l3 * 64;
        let l3_last_l2 = l3_first_l2 + 64;
        let first_l2 = ((start / SAMPLES_PER_L2_BIT) as usize).max(l3_first_l2);
        let last_l2 = ((end - 1) / SAMPLES_PER_L2_BIT) as usize;
        let l2_end = (last_l2 + 1).min(l3_last_l2);
        let mut l2_bits = *levels.l2_toggle.get(l3)?;
        l2_bits &= range_mask(first_l2 - l3_first_l2, l2_end - l3_first_l2);

        while let Some(l2_offset) = nonzero_trailing_bit(l2_bits) {
            l2_bits &= l2_bits - 1;
            let l2 = l3_first_l2 + l2_offset;
            let l2_first_l1 = l2 * 64;
            let l2_last_l1 = l2_first_l1 + 64;
            let first_l1 = ((start / SAMPLES_PER_L1_BIT) as usize).max(l2_first_l1);
            let last_l1 = ((end - 1) / SAMPLES_PER_L1_BIT) as usize;
            let l1_end = (last_l1 + 1).min(l2_last_l1);
            let mut l1_bits = *levels.l1_toggle.get(l2)?;
            l1_bits &= range_mask(first_l1 - l2_first_l1, l1_end - l2_first_l1);
            if let Some(l1_offset) = nonzero_trailing_bit(l1_bits) {
                return Some(l2_first_l1 + l1_offset);
            }
        }
    }
    None
}

fn nonzero_trailing_bit(word: u64) -> Option<usize> {
    (word != 0).then(|| word.trailing_zeros() as usize)
}

#[allow(clippy::too_many_arguments)]
fn append_raw_transitions(
    data: &[u8],
    block_start: u64,
    local_start: u64,
    local_end: u64,
    previous_block_last: Option<bool>,
    max_transitions: usize,
    output: &mut Vec<CaptureTransition>,
) {
    if local_start >= local_end || output.len() >= max_transitions {
        return;
    }

    let word_index = local_start as usize / 64;
    let word_start = word_index * 64;
    let word = load_le_word(data, word_index);
    let entering = if word_start > 0 {
        packed_bit(data, word_start - 1)
    } else {
        previous_block_last.unwrap_or(word & 1 != 0)
    };
    let lo = local_start as usize - word_start;
    let hi = local_end as usize - word_start;
    let mut toggles = word ^ ((word << 1) | entering as u64);
    toggles &= range_mask(lo, hi);

    while output.len() < max_transitions {
        let Some(bit_index) = nonzero_trailing_bit(toggles) else {
            break;
        };
        toggles &= toggles - 1;
        output.push(CaptureTransition {
            sample: block_start + (word_start + bit_index) as u64,
            value: bit(word, bit_index),
        });
    }
}

impl<R: BlockCaptureSource> CaptureIndex for IndexSampler<R> {
    fn display_name(&self) -> String {
        self.display_name()
    }
    fn index_identity(&self) -> SourceIdentity {
        self.index_identity()
    }
    fn header(&self) -> &CaptureMetadata {
        self.header()
    }
    fn build_profile(&self) -> Option<CaptureIndexBuildProfile> {
        self.build_profile()
    }
    fn capture_duration_us(&self) -> f64 {
        self.capture_duration_us()
    }
    fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        self.sampled_window(channels, start_sample, end_sample, target_points)
    }

    fn packed_block(&mut self, channel: usize, block: u64) -> Result<Option<BlockData>> {
        IndexSampler::packed_block(self, channel, block).map(Some)
    }
}

#[cfg(test)]
mod reader_tests {
    use std::sync::Arc;
    use std::thread::JoinHandle;

    use super::*;
    use crate::capture::{CaptureFingerprint, CaptureSource};
    use crate::{WorkExecutorTask, WorkTask};

    struct SpawnExecutor;

    impl WorkExecutor for SpawnExecutor {
        fn available_parallelism(&self) -> usize {
            2
        }

        fn submit(&self, task: WorkExecutorTask) -> std::result::Result<Box<dyn WorkTask>, String> {
            std::thread::Builder::new()
                .name("waveform-index-profile-test".to_owned())
                .spawn(task)
                .map(|handle| Box::new(JoinTask(Some(handle))) as Box<dyn WorkTask>)
                .map_err(|error| error.to_string())
        }
    }

    struct JoinTask(Option<JoinHandle<()>>);

    impl WorkTask for JoinTask {
        fn is_finished(&self) -> bool {
            self.0.as_ref().is_none_or(JoinHandle::is_finished)
        }

        fn wait(mut self: Box<Self>) {
            if let Some(handle) = self.0.take() {
                handle
                    .join()
                    .expect("waveform index worker should not panic");
            }
        }
    }

    #[derive(Clone)]
    struct TestDataSource {
        metadata: CaptureMetadata,
        blocks: Arc<Vec<Vec<Vec<u8>>>>,
        identity: SourceIdentity,
    }

    struct TestReader {
        metadata: CaptureMetadata,
        blocks: Arc<Vec<Vec<Vec<u8>>>>,
    }

    impl CaptureDataSource for TestDataSource {
        type Reader = TestReader;

        fn open_reader(&self) -> Result<Self::Reader> {
            Ok(TestReader {
                metadata: self.metadata.clone(),
                blocks: Arc::clone(&self.blocks),
            })
        }

        fn metadata(&self) -> &CaptureMetadata {
            &self.metadata
        }

        fn fingerprint(&self) -> CaptureFingerprint {
            CaptureFingerprint { revision: 1 }
        }

        fn index_identity(&self) -> Option<SourceIdentity> {
            Some(self.identity)
        }

        fn display_name(&self) -> String {
            "incremental fixture".to_owned()
        }
    }

    impl CaptureSource for TestReader {
        fn metadata(&self) -> &CaptureMetadata {
            &self.metadata
        }

        fn read_sample(&mut self, channel: usize, position: u64) -> Result<bool> {
            let block = position / self.metadata.samples_per_block;
            let offset = (position % self.metadata.samples_per_block) as usize;
            self.read_packed_block(channel, block)
                .map(|data| packed_bit(&data, offset))
        }
    }

    impl BlockCaptureSource for TestReader {
        fn read_packed_block(&mut self, channel: usize, block: u64) -> Result<BlockData> {
            self.blocks
                .get(channel)
                .and_then(|blocks| blocks.get(block as usize))
                .cloned()
                .map(BlockData::from)
                .ok_or(Error::InvalidBlock(block))
        }
    }

    #[test]
    fn raw_block_cache_reuses_backing_and_evicts_the_oldest_block() {
        let mut cache = RawBlockCache::default();
        for block in 0..RAW_BLOCK_CACHE_CAPACITY as u64 {
            cache.insert((0, block), BlockData::from(vec![block as u8]));
        }

        let recent = cache.get((0, 0)).expect("first block should be cached");
        let recent_again = cache.get((0, 0)).expect("first block should be reused");
        assert!(recent.shares_backing(&recent_again));

        cache.insert(
            (0, RAW_BLOCK_CACHE_CAPACITY as u64),
            BlockData::from(vec![0xff]),
        );
        assert!(cache.get((0, 1)).is_none());
        assert!(cache.get((0, 0)).is_some());
    }

    #[test]
    fn cold_open_reports_index_build_stage_metrics() {
        let source = TestDataSource {
            metadata: CaptureMetadata {
                total_probes: 1,
                samplerate: "1 MHz".to_owned(),
                samplerate_hz: 1_000_000.0,
                sample_period: 0.000_001,
                total_samples: 8,
                total_blocks: 1,
                samples_per_block: 8,
                probe_names: vec!["D0".to_owned()],
                trigger_sample: None,
            },
            blocks: Arc::new(vec![vec![vec![0x55]]]),
            identity: SourceIdentity::from_bytes([30; 32]),
        };
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let sampler = IndexSampler::<TestReader>::open_data_source_with_executor_and_progress(
            source.clone(),
            Arc::clone(&repository),
            Arc::new(InlineWorkExecutor),
            |_| true,
        )
        .expect("cold index build should succeed");
        let profile = sampler
            .build_profile()
            .expect("cold index build should expose its profile");
        assert_eq!(profile.blocks, 1);
        assert_eq!(profile.packed_bytes, 1);

        let reopened = IndexSampler::<TestReader>::open_existing_data_source(source, repository)
            .expect("published index should reopen")
            .expect("published index should exist");
        assert_eq!(reopened.build_profile(), None);
    }

    #[test]
    fn parallel_open_uses_bounded_workers_and_preserves_index_results() {
        let source = TestDataSource {
            metadata: CaptureMetadata {
                total_probes: 2,
                samplerate: "1 MHz".to_owned(),
                samplerate_hz: 1_000_000.0,
                sample_period: 0.000_001,
                total_samples: 16,
                total_blocks: 2,
                samples_per_block: 8,
                probe_names: vec!["D0".to_owned(), "D1".to_owned()],
                trigger_sample: None,
            },
            blocks: Arc::new(vec![
                vec![vec![0x0f], vec![0xf0]],
                vec![vec![0x55], vec![0xaa]],
            ]),
            identity: SourceIdentity::from_bytes([32; 32]),
        };
        let mut sampler = IndexSampler::<TestReader>::open_data_source_with_executor_and_progress(
            source,
            Arc::new(MemoryArtifactRepository::new()),
            Arc::new(SpawnExecutor),
            |_| true,
        )
        .expect("parallel index build should succeed");
        let profile = sampler
            .build_profile()
            .expect("parallel index build should expose its profile");
        assert_eq!(profile.workers, 2);
        assert_eq!(profile.blocks, 4);
        assert_eq!(profile.handoff_copy_ns, 0);

        let window = sampler
            .sampled_window(&[0, 1], 0, 16, 16)
            .expect("parallel index should remain queryable");
        assert_eq!(window.channels.len(), 2);
        assert!(!window.channels[0].transitions.is_empty());
        assert!(!window.channels[1].transitions.is_empty());
    }

    #[test]
    fn incremental_open_yields_once_per_channel_block_before_publication() {
        let source = TestDataSource {
            metadata: CaptureMetadata {
                total_probes: 2,
                samplerate: "1 MHz".to_owned(),
                samplerate_hz: 1_000_000.0,
                sample_period: 0.000_001,
                total_samples: 16,
                total_blocks: 2,
                samples_per_block: 8,
                probe_names: vec!["D0".to_owned(), "D1".to_owned()],
                trigger_sample: None,
            },
            blocks: Arc::new(vec![
                vec![vec![0x0f], vec![0xf0]],
                vec![vec![0x55], vec![0xaa]],
            ]),
            identity: SourceIdentity::from_bytes([31; 32]),
        };
        let repository = Arc::new(MemoryArtifactRepository::new());
        let mut task = IndexSampler::<TestReader>::begin_open_data_source(source, repository)
            .expect("incremental preparation should start");

        for completed in 1..=4 {
            assert!(matches!(
                task.step().unwrap(),
                CaptureIndexOpenStep::Progress(CaptureIndexBuildProgress {
                    completed: actual,
                    total: 4,
                }) if actual == completed
            ));
        }
        let CaptureIndexOpenStep::Ready(mut index) = task.step().unwrap() else {
            panic!("the completed task should publish its index");
        };
        assert_eq!(index.packed_block(1, 1).unwrap().unwrap().as_ref(), [0xaa]);
    }
}
