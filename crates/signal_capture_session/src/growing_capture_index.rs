use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use signal_artifacts::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, RepositoryError, SourceIdentity,
};
use signal_capture::{
    CaptureIndex, CaptureMetadata, CaptureSampledWindow, Error, Result,
    WaveformSummary as GroupSummary, WaveformSummaryGrid as SummaryGrid, exact_window_sample_limit,
    sample_waveform_summary_channel as sample_summary_channel,
    select_waveform_summary_resolution as select_summary_resolution,
};
use signal_runtime::WorkExecutor;

use crate::{
    CaptureCursorItem, CaptureRandomReader, CaptureStore, CaptureStoreCursor, FinalizedCapture,
};

const LEAF_SAMPLES: u64 = 64;
const FAN_OUT: usize = 64;
const QUERY_WAIT: Duration = Duration::from_millis(50);
const TIER_PAGE_RECORDS: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WaveformRecord {
    start_sample: u64,
    end_sample: u64,
    first: bool,
    last: bool,
    activity: bool,
}

impl WaveformRecord {
    fn combine(records: &[Self]) -> Self {
        let first = records[0];
        let last = records[records.len() - 1];
        let boundary_activity = records.windows(2).any(|pair| pair[0].last != pair[1].first);
        Self {
            start_sample: first.start_sample,
            end_sample: last.end_sample,
            first: first.first,
            last: last.last,
            activity: boundary_activity || records.iter().any(|record| record.activity),
        }
    }

    fn flags(self) -> u8 {
        u8::from(self.first) | (u8::from(self.last) << 1) | (u8::from(self.activity) << 2)
    }

    fn from_flags(flags: u8, start_sample: u64, end_sample: u64) -> Self {
        Self {
            start_sample,
            end_sample,
            first: flags & 1 != 0,
            last: flags & 2 != 0,
            activity: flags & 4 != 0,
        }
    }
}

struct WaveformTier {
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    channel: usize,
    tier: usize,
    span: u64,
    records: u64,
    page: Vec<u8>,
    pending: Vec<WaveformRecord>,
}

impl WaveformTier {
    fn create(
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        channel: usize,
        tier: usize,
        span: u64,
    ) -> Self {
        Self {
            repository,
            identity,
            channel,
            tier,
            span,
            records: 0,
            page: Vec::with_capacity(TIER_PAGE_RECORDS),
            pending: Vec::with_capacity(FAN_OUT),
        }
    }

    fn extend(&mut self, records: &[WaveformRecord]) -> Result<Vec<WaveformRecord>> {
        let flags = records
            .iter()
            .map(|record| record.flags())
            .collect::<Vec<_>>();
        for flag in flags {
            self.page.push(flag);
            self.records = self.records.saturating_add(1);
            if self.page.len() == TIER_PAGE_RECORDS {
                self.publish_page()?;
                self.page.clear();
            }
        }

        let mut folded = Vec::with_capacity((self.pending.len() + records.len()) / FAN_OUT);
        for &record in records {
            self.pending.push(record);
            if self.pending.len() == FAN_OUT {
                folded.push(WaveformRecord::combine(&self.pending));
                self.pending.clear();
            }
        }
        Ok(folded)
    }

    fn flush(&mut self) -> Result<()> {
        self.publish_page()
    }

    fn snapshot(&self) -> WaveformTierSnapshot {
        WaveformTierSnapshot {
            repository: Arc::clone(&self.repository),
            identity: self.identity,
            channel: self.channel,
            tier: self.tier,
            span: self.span,
            records: self.records,
            pending: self.pending.clone(),
        }
    }

    fn publish_page(&self) -> Result<()> {
        if self.page.is_empty() {
            return Ok(());
        }
        let page = (self.records - 1) / TIER_PAGE_RECORDS as u64;
        publish_page(
            self.repository.as_ref(),
            tier_page_key(self.identity, self.channel, self.tier, page)?,
            &self.page,
        )
    }
}

#[derive(Clone)]
struct WaveformTierSnapshot {
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    channel: usize,
    tier: usize,
    span: u64,
    records: u64,
    pending: Vec<WaveformRecord>,
}

impl WaveformTierSnapshot {
    fn read_range(&self, first: u64, last: u64) -> Result<Vec<WaveformRecord>> {
        if first >= last || last > self.records {
            return Ok(Vec::new());
        }
        let count = usize::try_from(last - first)
            .map_err(|_| Error::ParseError("waveform summary window is too large".into()))?;
        let mut flags = vec![0_u8; count];
        let mut copied = 0_usize;
        while copied < flags.len() {
            let record = first + copied as u64;
            let page = record / TIER_PAGE_RECORDS as u64;
            let page_offset = (record % TIER_PAGE_RECORDS as u64) as usize;
            let bytes = read_page(
                self.repository.as_ref(),
                &tier_page_key(self.identity, self.channel, self.tier, page)?,
            )?;
            let available = (flags.len() - copied).min(bytes.len().saturating_sub(page_offset));
            if available == 0 {
                return Err(Error::ParseError(
                    "growing waveform page is truncated".into(),
                ));
            }
            flags[copied..copied + available]
                .copy_from_slice(&bytes[page_offset..page_offset + available]);
            copied += available;
        }
        let mut records = Vec::with_capacity(count);
        for (relative, flags) in flags.into_iter().enumerate() {
            let index = first + relative as u64;
            let start_sample = index.saturating_mul(self.span);
            records.push(WaveformRecord::from_flags(
                flags,
                start_sample,
                start_sample.saturating_add(self.span),
            ));
        }
        Ok(records)
    }
}

struct WaveformMipmap {
    repository: Arc<dyn ArtifactRepository>,
    identity: SourceIdentity,
    channel: usize,
    tiers: Vec<WaveformTier>,
}

impl WaveformMipmap {
    fn new(
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
        channel: usize,
    ) -> Self {
        Self {
            repository,
            identity,
            channel,
            tiers: Vec::new(),
        }
    }

    fn push(&mut self, record: WaveformRecord) -> Result<()> {
        self.extend_at(0, &[record])
    }

    fn extend(&mut self, records: &[WaveformRecord]) -> Result<()> {
        self.extend_at(0, records)
    }

    fn extend_at(&mut self, tier: usize, records: &[WaveformRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        if tier == self.tiers.len() {
            let span = tier_span(tier)
                .ok_or_else(|| Error::ParseError("growing waveform tier span overflow".into()))?;
            self.tiers.push(WaveformTier::create(
                Arc::clone(&self.repository),
                self.identity,
                self.channel,
                tier,
                span,
            ));
        }
        let folded = self.tiers[tier].extend(records)?;
        if !folded.is_empty() {
            self.extend_at(tier + 1, &folded)?;
        }
        Ok(())
    }

    fn resident_records(&self) -> usize {
        self.tiers.iter().map(|tier| tier.pending.len()).sum()
    }

    fn flush(&mut self) -> Result<()> {
        for tier in &mut self.tiers {
            tier.flush()?;
        }
        Ok(())
    }

    fn snapshot(&self) -> WaveformMipmapSnapshot {
        WaveformMipmapSnapshot {
            tiers: self.tiers.iter().map(WaveformTier::snapshot).collect(),
        }
    }
}

#[derive(Clone)]
struct WaveformMipmapSnapshot {
    tiers: Vec<WaveformTierSnapshot>,
}

impl WaveformMipmapSnapshot {
    fn sampled_records(
        &self,
        start_sample: u64,
        end_sample: u64,
        resolution_samples: u64,
        target_points: usize,
        tail: Option<WaveformRecord>,
    ) -> Result<(Vec<WaveformRecord>, u64)> {
        let populated = self
            .tiers
            .iter()
            .enumerate()
            .filter(|(_, tier)| tier.records > 0)
            .map(|(tier, summary)| (tier, summary.span))
            .collect::<Vec<_>>();
        let selected_span = select_summary_resolution(
            resolution_samples,
            target_points,
            populated.iter().map(|(_, span)| *span),
        );
        if let Some(selected) = populated
            .iter()
            .position(|(_, span)| Some(*span) == selected_span)
        {
            let (tier_index, span) = populated[selected];
            let tier = &self.tiers[tier_index];
            let first = (start_sample / span).min(tier.records);
            let last = end_sample.div_ceil(span).min(tier.records);
            let mut result = tier.read_range(first, last)?;
            self.append_uncovered_tail(tier_index, start_sample, end_sample, &mut result);
            if let Some(tail) = tail
                && tail.end_sample > start_sample
                && tail.start_sample < end_sample
            {
                result.push(tail);
            }
            return Ok((result, span));
        }
        Ok((
            tail.into_iter()
                .filter(|tail| tail.end_sample > start_sample && tail.start_sample < end_sample)
                .collect(),
            LEAF_SAMPLES,
        ))
    }

    fn append_uncovered_tail(
        &self,
        tier_index: usize,
        start_sample: u64,
        end_sample: u64,
        output: &mut Vec<WaveformRecord>,
    ) {
        if tier_index == 0 {
            return;
        }
        let uncovered = (0..tier_index)
            .rev()
            .flat_map(|lower| self.tiers[lower].pending.iter().copied())
            .filter(|record| record.end_sample > start_sample && record.start_sample < end_sample)
            .collect::<Vec<_>>();
        if !uncovered.is_empty() {
            output.push(WaveformRecord::combine(&uncovered));
        }
    }
}

fn tier_span(tier: usize) -> Option<u64> {
    (FAN_OUT as u64)
        .checked_pow(tier as u32)
        .and_then(|scale| LEAF_SAMPLES.checked_mul(scale))
}

fn capture_identity(session_id: crate::CaptureSessionId) -> SourceIdentity {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"growing-waveform-v1");
    hasher.update(&session_id.get().to_le_bytes());
    SourceIdentity::from_bytes(*hasher.finalize().as_bytes())
}

fn tier_page_key(
    identity: SourceIdentity,
    channel: usize,
    tier: usize,
    page: u64,
) -> Result<ArtifactKey> {
    let namespace = ArtifactNamespace::new("growing-waveform-page-v1").map_err(repository_error)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(namespace.as_str().as_bytes());
    hasher.update(identity.as_bytes());
    hasher.update(&(channel as u64).to_le_bytes());
    hasher.update(&(tier as u64).to_le_bytes());
    hasher.update(&page.to_le_bytes());
    Ok(ArtifactKey::new(
        namespace,
        SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
    ))
}

fn publish_page(repository: &dyn ArtifactRepository, key: ArtifactKey, bytes: &[u8]) -> Result<()> {
    let mut writer = repository.begin_write(key).map_err(repository_error)?;
    writer.write_at(0, bytes).map_err(repository_error)?;
    writer
        .truncate(bytes.len() as u64)
        .map_err(repository_error)?;
    writer.flush().map_err(repository_error)?;
    writer.publish().map_err(repository_error)
}

fn read_page(repository: &dyn ArtifactRepository, key: &ArtifactKey) -> Result<Vec<u8>> {
    let mut reader = repository
        .open(key)
        .map_err(repository_error)?
        .ok_or_else(|| Error::ParseError("growing waveform page is missing".into()))?;
    let length = usize::try_from(reader.len().map_err(repository_error)?)
        .map_err(|_| Error::ParseError("growing waveform page is too large".into()))?;
    let mut bytes = vec![0_u8; length];
    let mut copied = 0;
    while copied < bytes.len() {
        let count = reader
            .read_at(copied as u64, &mut bytes[copied..])
            .map_err(repository_error)?;
        if count == 0 {
            return Err(Error::ParseError(
                "growing waveform page is truncated".into(),
            ));
        }
        copied += count;
    }
    Ok(bytes)
}

fn repository_error(error: RepositoryError) -> Error {
    Error::ParseError(error.to_string())
}

#[derive(Clone, Copy, Debug)]
struct ActiveRecord {
    start_sample: u64,
    first: bool,
    last: bool,
    activity: bool,
}

impl ActiveRecord {
    fn new(sample: u64, value: bool) -> Self {
        Self {
            start_sample: sample,
            first: value,
            last: value,
            activity: false,
        }
    }

    fn push(&mut self, value: bool) {
        self.activity |= value != self.last;
        self.last = value;
    }

    fn finish(self, end_sample: u64) -> WaveformRecord {
        WaveformRecord {
            start_sample: self.start_sample,
            end_sample,
            first: self.first,
            last: self.last,
            activity: self.activity,
        }
    }
}

struct SummaryBuilder {
    active: Vec<Option<ActiveRecord>>,
    next_sample: u64,
}

impl SummaryBuilder {
    fn new(channels: usize) -> Self {
        Self {
            active: vec![None; channels],
            next_sample: 0,
        }
    }

    fn append_chunk(&mut self, chunk: &crate::CaptureChunk) -> Result<Vec<Vec<WaveformRecord>>> {
        if chunk.start_sample() != self.next_sample || chunk.channels().len() != self.active.len() {
            return Err(Error::ParseError(
                "growing waveform received a discontinuous capture chunk".into(),
            ));
        }
        if self.active.len() > 64 {
            return self.append_chunk_scalar(chunk);
        }
        let mut completed = vec![Vec::new(); self.active.len()];
        let mut relative = 0_u64;
        while relative < chunk.sample_count() {
            let sample = chunk.start_sample() + relative;
            let leaf_remaining = LEAF_SAMPLES - sample % LEAF_SAMPLES;
            let sample_count = leaf_remaining.min(chunk.sample_count() - relative);
            let (first, last, activity) = summary_masks(chunk, relative, sample_count);
            for (channel, active) in self.active.iter_mut().enumerate() {
                let first = first & (1 << channel) != 0;
                let last = last & (1 << channel) != 0;
                let activity = activity & (1 << channel) != 0;
                match active {
                    Some(record) => {
                        record.activity |= record.last != first || activity;
                        record.last = last;
                    }
                    None => {
                        *active = Some(ActiveRecord {
                            start_sample: sample,
                            first,
                            last,
                            activity,
                        });
                    }
                }
            }
            relative += sample_count;
            let end_sample = sample + sample_count;
            if end_sample.is_multiple_of(LEAF_SAMPLES) {
                for (channel, active) in self.active.iter_mut().enumerate() {
                    completed[channel].push(
                        active
                            .take()
                            .expect("every channel has an active summary")
                            .finish(end_sample),
                    );
                }
            }
        }
        self.next_sample = chunk.end_sample();
        Ok(completed)
    }

    fn append_chunk_scalar(
        &mut self,
        chunk: &crate::CaptureChunk,
    ) -> Result<Vec<Vec<WaveformRecord>>> {
        let mut completed = vec![Vec::new(); self.active.len()];
        for relative in 0..chunk.sample_count() {
            let sample = chunk.start_sample() + relative;
            for (channel, active) in self.active.iter_mut().enumerate() {
                let value = chunk
                    .packed_level(relative, channel)
                    .expect("validated capture chunk contains every channel sample");
                match active {
                    Some(record) => record.push(value),
                    None => *active = Some(ActiveRecord::new(sample, value)),
                }
            }
            let end_sample = sample + 1;
            if end_sample.is_multiple_of(LEAF_SAMPLES) {
                for (channel, active) in self.active.iter_mut().enumerate() {
                    completed[channel].push(
                        active
                            .take()
                            .expect("every channel has an active summary")
                            .finish(end_sample),
                    );
                }
            }
        }
        self.next_sample = chunk.end_sample();
        Ok(completed)
    }

    fn active_records(&self) -> Vec<Option<WaveformRecord>> {
        self.active
            .iter()
            .map(|active| active.map(|record| record.finish(self.next_sample)))
            .collect()
    }

    fn finish(&mut self) -> Vec<Vec<WaveformRecord>> {
        let mut completed = vec![Vec::new(); self.active.len()];
        for (channel, active) in self.active.iter_mut().enumerate() {
            if let Some(active) = active.take() {
                completed[channel].push(active.finish(self.next_sample));
            }
        }
        completed
    }
}

fn summary_masks(chunk: &crate::CaptureChunk, start: u64, sample_count: u64) -> (u64, u64, u64) {
    debug_assert!(sample_count > 0 && sample_count <= LEAF_SAMPLES);
    let channels = chunk.channels().len();
    let first_bit = start as usize * channels;
    let first = packed_bits(chunk, first_bit, channels);
    let last = packed_bits(
        chunk,
        (start + sample_count - 1) as usize * channels,
        channels,
    );
    let mut activity = 0_u64;
    let comparison_bits = (sample_count as usize - 1) * channels;
    let mut offset = 0_usize;
    while offset < comparison_bits {
        let bits = (comparison_bits - offset).min(64);
        let current = packed_bits(chunk, first_bit + channels + offset, bits);
        let previous = packed_bits(chunk, first_bit + offset, bits);
        let mut differences = current ^ previous;
        while differences != 0 {
            let bit = differences.trailing_zeros() as usize;
            activity |= 1 << ((offset + bit) % channels);
            differences &= differences - 1;
        }
        if activity == channel_mask(channels) {
            break;
        }
        offset += bits;
    }
    (first, last, activity)
}

fn packed_bits(chunk: &crate::CaptureChunk, relative_bit: usize, bit_count: usize) -> u64 {
    debug_assert!(bit_count <= 64);
    if bit_count == 0 {
        return 0;
    }
    let crate::CaptureChunkPayload::PackedLsbFirst { bytes, bit_offset } = chunk.payload();
    let absolute_bit = usize::from(*bit_offset) + relative_bit;
    let first_byte = absolute_bit / 8;
    let shift = absolute_bit % 8;
    let needed_bytes = (shift + bit_count).div_ceil(8);
    let mut packed = 0_u128;
    for (index, byte) in bytes.as_slice()[first_byte..]
        .iter()
        .take(needed_bytes)
        .enumerate()
    {
        packed |= u128::from(*byte) << (index * 8);
    }
    let mask = if bit_count == 64 {
        u64::MAX
    } else {
        (1_u64 << bit_count) - 1
    };
    ((packed >> shift) as u64) & mask
}

fn channel_mask(channels: usize) -> u64 {
    if channels == 64 {
        u64::MAX
    } else {
        (1_u64 << channels) - 1
    }
}

struct GrowingState {
    channels: Vec<WaveformMipmap>,
    tails: Vec<Option<WaveformRecord>>,
    indexed_samples: u64,
    committed_chunks: u64,
    generation: u64,
    trigger_sample: Option<u64>,
    complete: bool,
    error: Option<String>,
}

impl GrowingState {
    fn new(
        channels: usize,
        repository: Arc<dyn ArtifactRepository>,
        identity: SourceIdentity,
    ) -> Self {
        Self {
            channels: (0..channels)
                .map(|channel| WaveformMipmap::new(Arc::clone(&repository), identity, channel))
                .collect(),
            tails: vec![None; channels],
            indexed_samples: 0,
            committed_chunks: 0,
            generation: 0,
            trigger_sample: None,
            complete: false,
            error: None,
        }
    }

    fn publish(
        &mut self,
        completed: Vec<Vec<WaveformRecord>>,
        tails: Vec<Option<WaveformRecord>>,
        indexed_samples: u64,
    ) -> Result<()> {
        for (mipmap, records) in self.channels.iter_mut().zip(completed) {
            mipmap.extend(&records)?;
        }
        for mipmap in &mut self.channels {
            mipmap.flush()?;
        }
        self.tails = tails;
        self.indexed_samples = indexed_samples;
        self.committed_chunks += 1;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    fn resident_summary_records(&self) -> usize {
        self.channels
            .iter()
            .map(WaveformMipmap::resident_records)
            .sum::<usize>()
            + self.tails.iter().flatten().count()
    }
}

/// Cloneable growing query handle. Its background owner follows a committed
/// store cursor; clones only read published summaries or exact raw windows.
pub struct GrowingCaptureIndex {
    display_name: String,
    identity: SourceIdentity,
    header: CaptureMetadata,
    store: CaptureStore,
    state: Arc<RwLock<GrowingState>>,
    random_reader: Option<CaptureRandomReader>,
}

impl Clone for GrowingCaptureIndex {
    fn clone(&self) -> Self {
        Self {
            display_name: self.display_name.clone(),
            identity: self.identity,
            header: self.header.clone(),
            store: self.store.clone(),
            state: Arc::clone(&self.state),
            random_reader: None,
        }
    }
}

impl GrowingCaptureIndex {
    /// Builds a growing index for a finalized capture using the same worker path as live capture.
    ///
    /// # Parameters
    /// - `capture`: Finalized authoritative capture to index.
    /// - `display_name`: Human-readable name for query consumers.
    /// - `sample_rate_hz`: Positive capture sample rate in hertz.
    /// - `probe_names`: Display names in capture channel order.
    /// - `work_executor`: Host capability that runs the indexing worker.
    pub fn rebuild(
        capture: &FinalizedCapture,
        display_name: impl Into<String>,
        sample_rate_hz: f64,
        probe_names: Vec<String>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<(Self, GrowingCaptureIndexWorker)> {
        Self::spawn(
            capture.store_handle(),
            display_name,
            sample_rate_hz,
            probe_names,
            work_executor,
        )
    }

    /// Starts a growing index that follows a live capture store cursor.
    ///
    /// The returned query is usable immediately; wide windows gain summaries
    /// as the worker consumes committed chunks.
    ///
    /// # Parameters
    ///
    /// - `store`: Authoritative capture store to follow.
    /// - `display_name`: Human-readable name for query consumers.
    /// - `sample_rate_hz`: Positive capture sample rate in hertz.
    /// - `probe_names`: Display names in descriptor channel order.
    /// - `work_executor`: Host capability that runs the indexing worker.
    pub fn spawn(
        store: CaptureStore,
        display_name: impl Into<String>,
        sample_rate_hz: f64,
        probe_names: Vec<String>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Result<(Self, GrowingCaptureIndexWorker)> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(Error::ParseError(
                "live capture sample rate must be positive".into(),
            ));
        }
        if probe_names.len() != store.descriptor().channels().len() {
            return Err(Error::ParseError(
                "live capture channel names do not match its channel table".into(),
            ));
        }
        let header = CaptureMetadata {
            total_probes: probe_names.len(),
            samplerate: format_sample_rate(sample_rate_hz),
            samplerate_hz: sample_rate_hz,
            sample_period: 1.0 / sample_rate_hz,
            total_samples: 0,
            total_blocks: 0,
            samples_per_block: LEAF_SAMPLES,
            probe_names,
            trigger_sample: None,
        };
        let identity = capture_identity(store.descriptor().session_id());
        let state = Arc::new(RwLock::new(GrowingState::new(
            header.total_probes,
            store.repository(),
            identity,
        )));
        let cursor = store
            .open_cursor()
            .map_err(|error| Error::ParseError(error.to_string()))?;
        let worker_state = Arc::clone(&state);
        let channels = header.total_probes;
        let completed = Arc::new(AtomicBool::new(false));
        let result = Arc::new(Mutex::new(None));
        let worker_completed = Arc::clone(&completed);
        let worker_result = Arc::clone(&result);
        work_executor
            .submit(Box::new(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_index_worker(cursor, worker_state, channels)
                }))
                .map_err(|_| Error::ParseError("live waveform index worker panicked".into()))
                .and_then(std::convert::identity);
                *worker_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(outcome);
                worker_completed.store(true, Ordering::Release);
            }))
            .map_err(Error::ParseError)?;
        let query = Self {
            display_name: display_name.into(),
            identity,
            header,
            store,
            state,
            random_reader: None,
        };
        Ok((query, GrowingCaptureIndexWorker { completed, result }))
    }

    fn snapshot_metadata(&self) -> CaptureMetadata {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        let mut metadata = self.header.clone();
        metadata.total_samples = state.indexed_samples;
        metadata.total_blocks = state.committed_chunks;
        metadata.trigger_sample = state.trigger_sample;
        metadata
    }

    /// Sets trigger sample.
    ///
    /// # Parameters
    /// - `sample`: Authoritative trigger sample to expose through metadata.
    pub fn set_trigger_sample(&self, sample: u64) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if state.trigger_sample != Some(sample) {
            state.trigger_sample = Some(sample);
            state.generation = state.generation.wrapping_add(1);
        }
    }

    /// Number of summary records retained in RAM. Historical records are in
    /// fixed-size tier-page artifacts beside the authoritative raw capture.
    pub fn resident_summary_records(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .resident_summary_records()
    }

    fn exact_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
    ) -> Result<CaptureSampledWindow> {
        if self.random_reader.is_none() {
            self.random_reader = Some(
                self.store
                    .open_random_reader()
                    .map_err(|error| Error::ParseError(error.to_string()))?,
            );
        }
        let mut window = self
            .random_reader
            .as_mut()
            .expect("random reader was initialized")
            .sampled_window(channels, start_sample, end_sample)?;
        for channel in &mut window.channels {
            channel.name = self.header.probe_names[channel.channel].clone();
        }
        Ok(window)
    }

    fn summary_window(
        &self,
        channels: &[usize],
        start_sample: u64,
        available_end_sample: u64,
        grid_end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        let snapshots = {
            let state = self.state.read().unwrap_or_else(|error| error.into_inner());
            if let Some(error) = &state.error {
                return Err(Error::ParseError(error.clone()));
            }
            let mut snapshots = Vec::with_capacity(channels.len());
            for &channel in channels {
                let Some(mipmap) = state.channels.get(channel) else {
                    return Err(Error::InvalidProbe(channel));
                };
                snapshots.push((channel, mipmap.snapshot(), state.tails[channel]));
            }
            snapshots
        };

        let mut sampled = Vec::with_capacity(channels.len());
        let mut sample_step = 1_u64;
        for (channel, mipmap, tail) in snapshots {
            let (records, group_samples) = mipmap.sampled_records(
                start_sample,
                available_end_sample,
                grid_end_sample.saturating_sub(start_sample),
                target_points,
                tail,
            )?;
            sample_step = sample_step.max(group_samples);
            let initial = records.first().is_some_and(|record| record.first);
            sampled.push(sample_summary_channel(
                channel,
                self.header.probe_names[channel].clone(),
                initial,
                SummaryGrid {
                    start_sample,
                    available_end_sample,
                    grid_end_sample,
                    target_points,
                },
                |visible_start, visible_end, fallback_first| {
                    Ok(records_range_summary(
                        &records,
                        visible_start,
                        visible_end,
                        fallback_first,
                    ))
                },
            )?);
        }
        Ok(CaptureSampledWindow {
            start_sample,
            end_sample: available_end_sample,
            sample_step,
            channels: sampled,
        })
    }
}

impl CaptureIndex for GrowingCaptureIndex {
    fn display_name(&self) -> String {
        self.display_name.clone()
    }

    fn index_identity(&self) -> SourceIdentity {
        self.identity
    }

    fn header(&self) -> &CaptureMetadata {
        &self.header
    }

    fn current_metadata(&self) -> CaptureMetadata {
        self.snapshot_metadata()
    }

    fn generation(&self) -> u64 {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .generation
    }

    fn is_complete(&self) -> bool {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .complete
    }

    fn capture_duration_us(&self) -> f64 {
        self.snapshot_metadata().duration_us()
    }

    fn sampled_window(
        &mut self,
        channels: &[usize],
        start_sample: u64,
        end_sample: u64,
        target_points: usize,
    ) -> Result<CaptureSampledWindow> {
        let metadata = self.snapshot_metadata();
        if metadata.total_samples == 0 {
            return Err(Error::OutOfBounds(end_sample));
        }
        let requested_end_sample = end_sample.max(start_sample.saturating_add(1));
        let start_sample = start_sample.min(metadata.total_samples - 1);
        let end_sample = end_sample.clamp(start_sample + 1, metadata.total_samples);
        let grid_end_sample = requested_end_sample.max(end_sample);
        if grid_end_sample.saturating_sub(start_sample) <= exact_window_sample_limit(target_points)
        {
            self.exact_window(channels, start_sample, end_sample)
        } else {
            self.summary_window(
                channels,
                start_sample,
                end_sample,
                grid_end_sample,
                target_points,
            )
        }
    }
}

pub struct GrowingCaptureIndexWorker {
    completed: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Result<()>>>>,
}

impl GrowingCaptureIndexWorker {
    /// Returns whether finished.
    pub fn is_finished(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    /// Waits for the background index worker and returns its final result.
    pub fn join(self) -> Result<()> {
        while !self.is_finished() {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .unwrap_or_else(|| {
                Err(Error::ParseError(
                    "live waveform index worker completed without a result".into(),
                ))
            })
    }
}

fn run_index_worker(
    mut cursor: crate::CaptureCursor,
    state: Arc<RwLock<GrowingState>>,
    channels: usize,
) -> Result<()> {
    let mut builder = SummaryBuilder::new(channels);
    loop {
        match cursor
            .wait_next(QUERY_WAIT)
            .map_err(|error| Error::ParseError(error.to_string()))
        {
            Ok(CaptureCursorItem::Chunk(chunk)) => {
                let completed = builder.append_chunk(&chunk)?;
                let tails = builder.active_records();
                if let Err(error) = state
                    .write()
                    .unwrap_or_else(|error| error.into_inner())
                    .publish(completed, tails, chunk.end_sample())
                {
                    let message = error.to_string();
                    let mut state = state.write().unwrap_or_else(|error| error.into_inner());
                    state.error = Some(message);
                    state.complete = true;
                    state.generation = state.generation.wrapping_add(1);
                    return Err(error);
                }
            }
            Ok(CaptureCursorItem::Pending) => {}
            Ok(CaptureCursorItem::End) => {
                let completed = builder.finish();
                let mut state = state.write().unwrap_or_else(|error| error.into_inner());
                for (mipmap, records) in state.channels.iter_mut().zip(completed) {
                    for record in records {
                        if let Err(error) = mipmap.push(record) {
                            state.error = Some(error.to_string());
                            state.complete = true;
                            state.generation = state.generation.wrapping_add(1);
                            return Err(error);
                        }
                    }
                }
                for mipmap in &mut state.channels {
                    if let Err(error) = mipmap.flush() {
                        state.error = Some(error.to_string());
                        state.complete = true;
                        state.generation = state.generation.wrapping_add(1);
                        return Err(error);
                    }
                }
                state.tails.fill(None);
                state.complete = true;
                state.generation = state.generation.wrapping_add(1);
                return Ok(());
            }
            Err(error) => {
                let message = error.to_string();
                let mut state = state.write().unwrap_or_else(|error| error.into_inner());
                state.error = Some(message);
                state.complete = true;
                state.generation = state.generation.wrapping_add(1);
                return Err(error);
            }
        }
    }
}

fn records_range_summary(
    records: &[WaveformRecord],
    start_sample: u64,
    end_sample: u64,
    fallback_first: bool,
) -> GroupSummary {
    let first_index = records.partition_point(|record| record.end_sample <= start_sample);
    let count = records[first_index..].partition_point(|record| record.start_sample < end_sample);
    let overlapping = &records[first_index..first_index + count];
    let Some(first_record) = overlapping.first() else {
        return GroupSummary {
            first: fallback_first,
            toggle: false,
            last: fallback_first,
        };
    };
    let last_record = overlapping[overlapping.len() - 1];
    let boundary_toggle = overlapping
        .windows(2)
        .any(|pair| pair[0].last != pair[1].first);
    GroupSummary {
        first: first_record.first,
        toggle: boundary_toggle || overlapping.iter().any(|record| record.activity),
        last: last_record.last,
    }
}

fn format_sample_rate(sample_rate_hz: f64) -> String {
    if sample_rate_hz >= 1_000_000_000.0 {
        format!("{:.3} GHz", sample_rate_hz / 1_000_000_000.0)
    } else if sample_rate_hz >= 1_000_000.0 {
        format!("{:.3} MHz", sample_rate_hz / 1_000_000.0)
    } else if sample_rate_hz >= 1_000.0 {
        format!("{:.3} kHz", sample_rate_hz / 1_000.0)
    } else {
        format!("{sample_rate_hz:.0} Hz")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use signal_artifacts::MemoryArtifactRepository;
    use signal_capture::{CaptureIndex, CaptureWaveformSegment};
    use signal_runtime::{CompletedWorkTask, WorkExecutor, WorkExecutorTask, WorkTask};

    use super::{FAN_OUT, GrowingCaptureIndex, LEAF_SAMPLES, summary_masks};
    use crate::{
        CaptureChannelId, CaptureChunk, CaptureChunkWriter, CaptureSessionId, CaptureStore,
        CaptureStoreConfig, CaptureStoreDescriptor,
    };

    struct SpawnWorkExecutor;

    impl WorkExecutor for SpawnWorkExecutor {
        fn available_parallelism(&self) -> usize {
            1
        }

        fn submit(&self, task: WorkExecutorTask) -> Result<Box<dyn WorkTask>, String> {
            std::thread::spawn(task);
            Ok(Box::new(CompletedWorkTask))
        }
    }

    fn worker_executor() -> Arc<dyn WorkExecutor> {
        Arc::new(SpawnWorkExecutor)
    }

    fn level_at(sample: u64, channel: usize) -> bool {
        (sample / (37 + channel as u64 * 11) + channel as u64).is_multiple_of(2)
    }

    fn chunk(
        session: CaptureSessionId,
        channels: Arc<[CaptureChannelId]>,
        sequence: u64,
        start_sample: u64,
        sample_count: u64,
    ) -> CaptureChunk {
        let bit_offset = ((sequence * 5 + 3) % 8) as u8;
        let bit_count = sample_count as usize * channels.len();
        let mut bytes = vec![0_u8; (usize::from(bit_offset) + bit_count).div_ceil(8)];
        for relative in 0..sample_count {
            for channel in 0..channels.len() {
                if level_at(start_sample + relative, channel) {
                    let bit =
                        usize::from(bit_offset) + relative as usize * channels.len() + channel;
                    bytes[bit / 8] |= 1 << (bit % 8);
                }
            }
        }
        CaptureChunk::packed_lsb_first(
            session,
            sequence,
            start_sample,
            sample_count,
            channels,
            bytes,
            bit_offset,
        )
        .unwrap()
    }

    fn wait_for_generation(index: &GrowingCaptureIndex, generation: u64) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while index.generation() < generation {
            assert!(Instant::now() < deadline, "growing index timed out");
            std::thread::yield_now();
        }
    }

    #[test]
    fn packed_summary_masks_match_scalar_levels_for_varied_channel_widths() {
        let session = CaptureSessionId::new(0x5a77);
        for channel_count in [1, 3, 6, 16, 19, 64] {
            let channels: Arc<[CaptureChannelId]> = (0..channel_count)
                .map(|channel| CaptureChannelId::new(format!("input:{channel}")))
                .collect::<Vec<_>>()
                .into();
            let chunk = chunk(session, channels, 5, 0, 137);
            for (start, sample_count) in [(0, 1), (0, 64), (1, 63), (63, 64), (70, 17)] {
                let (first, last, activity) = summary_masks(&chunk, start, sample_count);
                for channel in 0..channel_count {
                    assert_eq!(
                        first & (1 << channel) != 0,
                        level_at(start, channel),
                        "first: {channel_count} channels, sample {start}, channel {channel}"
                    );
                    assert_eq!(
                        last & (1 << channel) != 0,
                        level_at(start + sample_count - 1, channel),
                        "last: {channel_count} channels, sample {start}, channel {channel}"
                    );
                    let expected_activity = (start + 1..start + sample_count)
                        .any(|sample| level_at(sample - 1, channel) != level_at(sample, channel));
                    assert_eq!(
                        activity & (1 << channel) != 0,
                        expected_activity,
                        "activity: {channel_count} channels, sample {start}, channel {channel}"
                    );
                }
            }
        }
    }

    fn expected_transitions(
        channel: usize,
        start_sample: u64,
        end_sample: u64,
    ) -> Vec<signal_capture::CaptureTransition> {
        let mut previous = level_at(start_sample, channel);
        let mut transitions = Vec::new();
        for sample in start_sample + 1..end_sample {
            let value = level_at(sample, channel);
            if value != previous {
                transitions.push(signal_capture::CaptureTransition { sample, value });
                previous = value;
            }
        }
        transitions
    }

    #[test]
    fn growing_query_is_visible_before_completion_and_matches_final_raw_and_summary_data() {
        let session = CaptureSessionId::new(0x71a3);
        let channels: Arc<[CaptureChannelId]> = vec![
            CaptureChannelId::new("bank-a:7"),
            CaptureChannelId::new("bank-c:2"),
        ]
        .into();
        let descriptor = CaptureStoreDescriptor::new(session, Arc::clone(&channels)).unwrap();
        let repository = Arc::new(MemoryArtifactRepository::new());
        let config = CaptureStoreConfig::new(repository.clone(), descriptor);
        let (store, mut writer) = CaptureStore::create(config).unwrap();
        let (mut index, worker) = GrowingCaptureIndex::spawn(
            store.clone(),
            "Growing test",
            1_000_000.0,
            vec!["A7".into(), "C2".into()],
            worker_executor(),
        )
        .unwrap();

        writer
            .append(chunk(session, Arc::clone(&channels), 0, 0, 75))
            .unwrap();
        wait_for_generation(&index, 1);
        assert!(!index.is_complete());
        assert_eq!(index.current_metadata().total_samples, 75);
        let live = index.sampled_window(&[0, 1], 0, 75, 75).unwrap();
        for channel in &live.channels {
            assert_eq!(channel.initial, level_at(0, channel.channel));
            assert_eq!(
                channel.transitions,
                expected_transitions(channel.channel, 0, 75)
            );
        }

        writer
            .append(chunk(session, Arc::clone(&channels), 1, 75, 5_000))
            .unwrap();
        writer
            .append(chunk(session, Arc::clone(&channels), 2, 5_075, 6_000))
            .unwrap();
        writer.finish().unwrap();
        drop(writer);
        worker.join().unwrap();
        store.finalize().unwrap();

        let total_samples = 11_075;
        assert!(index.is_complete());
        assert_eq!(index.current_metadata().total_samples, total_samples);
        let exact = index
            .sampled_window(&[0, 1], 0, total_samples, total_samples as usize)
            .unwrap();
        for channel in &exact.channels {
            assert_eq!(channel.initial, level_at(0, channel.channel));
            assert_eq!(
                channel.transitions,
                expected_transitions(channel.channel, 0, total_samples)
            );
        }

        let summary = index.sampled_window(&[0, 1], 0, total_samples, 1).unwrap();
        assert!(summary.sample_step > 1);
        for channel in &summary.channels {
            let mut next = 0;
            for segment in &channel.waveform {
                let (start, end) = match *segment {
                    CaptureWaveformSegment::Level {
                        start_sample,
                        end_sample,
                        value,
                    } => {
                        assert!(
                            (start_sample..end_sample)
                                .all(|sample| level_at(sample, channel.channel) == value)
                        );
                        (start_sample, end_sample)
                    }
                    CaptureWaveformSegment::Activity {
                        start_sample,
                        end_sample,
                        first,
                        last,
                    } => {
                        assert_eq!(first, level_at(start_sample, channel.channel));
                        assert_eq!(last, level_at(end_sample - 1, channel.channel));
                        assert!((start_sample + 1..end_sample).any(|sample| {
                            level_at(sample - 1, channel.channel)
                                != level_at(sample, channel.channel)
                        }));
                        (start_sample, end_sample)
                    }
                    CaptureWaveformSegment::Edge { .. } => {
                        panic!("coarse growing summaries use level/activity segments")
                    }
                };
                assert_eq!(start, next);
                next = end;
            }
            assert_eq!(next, total_samples);
        }
    }

    #[test]
    fn long_capture_keeps_only_bounded_summary_fold_state_in_memory() {
        let session = CaptureSessionId::new(0x71a4);
        let channels: Arc<[CaptureChannelId]> = vec![
            CaptureChannelId::new("bank-a:0"),
            CaptureChannelId::new("bank-a:1"),
        ]
        .into();
        let descriptor = CaptureStoreDescriptor::new(session, Arc::clone(&channels)).unwrap();
        let repository = Arc::new(MemoryArtifactRepository::new());
        let config = CaptureStoreConfig::new(repository.clone(), descriptor);
        let (store, mut writer) = CaptureStore::create(config).unwrap();
        let (mut index, worker) = GrowingCaptureIndex::spawn(
            store.clone(),
            "Bounded summary test",
            100_000_000.0,
            vec!["A0".into(), "A1".into()],
            worker_executor(),
        )
        .unwrap();
        let mut start = 0_u64;
        for sequence in 0..256 {
            writer
                .append(chunk(session, Arc::clone(&channels), sequence, start, 4096))
                .unwrap();
            start += 4096;
        }
        writer.finish().unwrap();
        drop(writer);
        worker.join().unwrap();
        store.finalize().unwrap();

        assert_eq!(index.current_metadata().total_samples, 1_048_576);
        assert!(
            index.resident_summary_records() <= channels.len() * FAN_OUT * 12,
            "summary RAM must be bounded by fold state, not capture duration"
        );
        assert_eq!(store.snapshot().resident_commit_records, 0);
        assert!(repository.used_bytes().unwrap() > 1_048_576 / LEAF_SAMPLES);
        let old_window = index.sampled_window(&[0, 1], 0, 4096, 8).unwrap();
        assert_eq!(old_window.start_sample, 0);
        assert_eq!(old_window.end_sample, 4096);

        // This range is above the exact-scan limit, but its 64-sample leaf
        // records still fit the display budget. A live query must not jump to
        // a capture-wide folded tier merely because that tier now exists.
        let fine_summary = index.sampled_window(&[0], 100_000, 200_000, 1_000).unwrap();
        assert_eq!(fine_summary.sample_step, 64);

        // Conversely, a capture-wide query must climb to a folded tier. Its
        // result is GPU/display work, so it must remain proportional to the
        // viewport budget rather than the number of 64-sample leaves.
        let coarse_summary = index.sampled_window(&[0], 0, 1_048_576, 100).unwrap();
        assert!(coarse_summary.sample_step >= 4_096);
        assert!(coarse_summary.channels[0].waveform.len() <= 200);
    }
}
