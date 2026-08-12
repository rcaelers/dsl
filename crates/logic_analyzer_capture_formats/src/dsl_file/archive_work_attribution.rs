use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use platform_artifacts::SourceIdentity;

/// Work attributed to one DSL archive consumer phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslArchiveWorkCounters {
    /// ZIP entries opened by name, including metadata-only inspection.
    pub archive_entries_opened: u64,
    /// Opened entries whose ZIP method requires decompression.
    pub compressed_entries_opened: u64,
    /// Compressed payload bytes represented by entries read to completion.
    pub compressed_bytes: u64,
    /// Expanded payload bytes returned to the DSL reader.
    pub expanded_bytes: u64,
    /// Compressed entries expanded to completion.
    pub decompressions: u64,
    /// Successful DSL block-cache lookups.
    pub block_cache_hits: u64,
    /// DSL block-cache lookups that required archive access.
    pub block_cache_misses: u64,
    /// Positional reads issued against the prepared source.
    pub source_reads: u64,
    /// Bytes returned by positional prepared-source reads.
    pub source_bytes: u64,
    /// Source reads overlapping a range read earlier in this attribution session.
    pub source_ranges_reread: u64,
    /// Bytes overlapping source ranges read earlier in this attribution session.
    pub source_reread_bytes: u64,
    /// Acquisitions of a shared archive lock that had to wait.
    pub archive_waits: u64,
    /// Nanoseconds spent waiting to acquire shared archive access.
    pub archive_wait_ns: u64,
}

impl DslArchiveWorkCounters {
    fn add_assign(&mut self, other: Self) {
        self.archive_entries_opened = self
            .archive_entries_opened
            .saturating_add(other.archive_entries_opened);
        self.compressed_entries_opened = self
            .compressed_entries_opened
            .saturating_add(other.compressed_entries_opened);
        self.compressed_bytes = self.compressed_bytes.saturating_add(other.compressed_bytes);
        self.expanded_bytes = self.expanded_bytes.saturating_add(other.expanded_bytes);
        self.decompressions = self.decompressions.saturating_add(other.decompressions);
        self.block_cache_hits = self.block_cache_hits.saturating_add(other.block_cache_hits);
        self.block_cache_misses = self
            .block_cache_misses
            .saturating_add(other.block_cache_misses);
        self.source_reads = self.source_reads.saturating_add(other.source_reads);
        self.source_bytes = self.source_bytes.saturating_add(other.source_bytes);
        self.source_ranges_reread = self
            .source_ranges_reread
            .saturating_add(other.source_ranges_reread);
        self.source_reread_bytes = self
            .source_reread_bytes
            .saturating_add(other.source_reread_bytes);
        self.archive_waits = self.archive_waits.saturating_add(other.archive_waits);
        self.archive_wait_ns = self.archive_wait_ns.saturating_add(other.archive_wait_ns);
    }
}

/// DSL archive work grouped by the source generation and consumer phase that caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DslArchiveWorkProfile {
    /// Stable identity of the prepared source generation being observed.
    pub source_identity: SourceIdentity,
    /// Header and archive-layout discovery.
    pub metadata: DslArchiveWorkCounters,
    /// Raw reads performed while constructing the finite waveform index.
    pub waveform_index: DslArchiveWorkCounters,
    /// Raw reads performed while delivering blocks to graph execution.
    pub runtime_delivery: DslArchiveWorkCounters,
    /// Exact/raw reads performed by interactive capture presentations.
    pub presentation_queries: DslArchiveWorkCounters,
}

impl DslArchiveWorkProfile {
    /// Returns counters summed across every consumer phase.
    pub fn total(&self) -> DslArchiveWorkCounters {
        let mut total = self.metadata;
        total.add_assign(self.waveform_index);
        total.add_assign(self.runtime_delivery);
        total.add_assign(self.presentation_queries);
        total
    }
}

/// Opt-in attribution session for one immutable DSL prepared-source generation.
///
/// Keep this handle alive while opening indexes, executing graph sources, and issuing viewer
/// queries. Dropping it disables collection; instrumentation is absent when no session is active.
pub struct DslArchiveWorkAttribution {
    state: Arc<AttributionState>,
}

impl DslArchiveWorkAttribution {
    /// Begins a fresh attribution session for `source_identity`.
    pub fn begin(source_identity: SourceIdentity) -> Self {
        let state = Arc::new(AttributionState::new(source_identity));
        active_sessions()
            .lock()
            .unwrap()
            .insert(source_identity, Arc::downgrade(&state));
        Self { state }
    }

    /// Returns a consistent snapshot of all work observed so far.
    pub fn snapshot(&self) -> DslArchiveWorkProfile {
        self.state.snapshot()
    }
}

impl Drop for DslArchiveWorkAttribution {
    fn drop(&mut self) {
        self.state.disable();
        let mut sessions = active_sessions().lock().unwrap();
        if sessions
            .get(&self.state.identity)
            .and_then(Weak::upgrade)
            .is_some_and(|active| Arc::ptr_eq(&active, &self.state))
        {
            sessions.remove(&self.state.identity);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ArchiveWorkPhase {
    Metadata,
    WaveformIndex,
    RuntimeDelivery,
    PresentationQuery,
}

#[derive(Clone)]
pub(crate) struct ArchiveWorkRecorder {
    state: Arc<AttributionState>,
    phase: ArchiveWorkPhase,
}

impl ArchiveWorkRecorder {
    pub(crate) fn record_entry_open(&self, compressed: bool) {
        self.state.record(self.phase, |counters| {
            counters.archive_entries_opened = counters.archive_entries_opened.saturating_add(1);
            if compressed {
                counters.compressed_entries_opened =
                    counters.compressed_entries_opened.saturating_add(1);
            }
        });
    }

    pub(crate) fn record_entry_read(
        &self,
        compressed_bytes: u64,
        expanded_bytes: u64,
        decompressed: bool,
    ) {
        self.state.record(self.phase, |counters| {
            counters.compressed_bytes = counters.compressed_bytes.saturating_add(compressed_bytes);
            counters.expanded_bytes = counters.expanded_bytes.saturating_add(expanded_bytes);
            if decompressed {
                counters.decompressions = counters.decompressions.saturating_add(1);
            }
        });
    }

    pub(crate) fn record_cache(&self, hit: bool) {
        self.state.record(self.phase, |counters| {
            if hit {
                counters.block_cache_hits = counters.block_cache_hits.saturating_add(1);
            } else {
                counters.block_cache_misses = counters.block_cache_misses.saturating_add(1);
            }
        });
    }

    pub(crate) fn record_source_read(&self, offset: u64, length: u64) {
        self.state.record_source_read(self.phase, offset, length);
    }

    pub(crate) fn record_archive_wait(&self, duration: Duration) {
        self.state.record(self.phase, |counters| {
            counters.archive_waits = counters.archive_waits.saturating_add(1);
            counters.archive_wait_ns = counters
                .archive_wait_ns
                .saturating_add(duration_ns(duration));
        });
    }
}

pub(crate) fn active_archive_work(
    identity: SourceIdentity,
    phase: ArchiveWorkPhase,
) -> Option<ArchiveWorkRecorder> {
    let mut sessions = active_sessions().lock().unwrap();
    let state = sessions.get(&identity).and_then(Weak::upgrade);
    if state.is_none() {
        sessions.remove(&identity);
    }
    state.map(|state| ArchiveWorkRecorder { state, phase })
}

struct AttributionState {
    identity: SourceIdentity,
    inner: Mutex<AttributionInner>,
}

impl AttributionState {
    fn new(identity: SourceIdentity) -> Self {
        Self {
            identity,
            inner: Mutex::new(AttributionInner::default()),
        }
    }

    fn disable(&self) {
        self.inner.lock().unwrap().enabled = false;
    }

    fn record(&self, phase: ArchiveWorkPhase, update: impl FnOnce(&mut DslArchiveWorkCounters)) {
        let mut inner = self.inner.lock().unwrap();
        if !inner.enabled {
            return;
        }
        update(inner.counters_mut(phase));
    }

    fn record_source_read(&self, phase: ArchiveWorkPhase, offset: u64, length: u64) {
        if length == 0 {
            return;
        }
        let end = offset.saturating_add(length);
        let mut inner = self.inner.lock().unwrap();
        if !inner.enabled {
            return;
        }
        let overlap = covered_bytes(&inner.source_ranges, offset..end);
        let counters = inner.counters_mut(phase);
        counters.source_reads = counters.source_reads.saturating_add(1);
        counters.source_bytes = counters.source_bytes.saturating_add(length);
        if overlap > 0 {
            counters.source_ranges_reread = counters.source_ranges_reread.saturating_add(1);
            counters.source_reread_bytes = counters.source_reread_bytes.saturating_add(overlap);
        }
        insert_covered_range(&mut inner.source_ranges, offset..end);
    }

    fn snapshot(&self) -> DslArchiveWorkProfile {
        let inner = self.inner.lock().unwrap();
        DslArchiveWorkProfile {
            source_identity: self.identity,
            metadata: inner.metadata,
            waveform_index: inner.waveform_index,
            runtime_delivery: inner.runtime_delivery,
            presentation_queries: inner.presentation_queries,
        }
    }
}

struct AttributionInner {
    enabled: bool,
    metadata: DslArchiveWorkCounters,
    waveform_index: DslArchiveWorkCounters,
    runtime_delivery: DslArchiveWorkCounters,
    presentation_queries: DslArchiveWorkCounters,
    source_ranges: Vec<Range<u64>>,
}

impl Default for AttributionInner {
    fn default() -> Self {
        Self {
            enabled: true,
            metadata: DslArchiveWorkCounters::default(),
            waveform_index: DslArchiveWorkCounters::default(),
            runtime_delivery: DslArchiveWorkCounters::default(),
            presentation_queries: DslArchiveWorkCounters::default(),
            source_ranges: Vec::new(),
        }
    }
}

impl AttributionInner {
    fn counters_mut(&mut self, phase: ArchiveWorkPhase) -> &mut DslArchiveWorkCounters {
        match phase {
            ArchiveWorkPhase::Metadata => &mut self.metadata,
            ArchiveWorkPhase::WaveformIndex => &mut self.waveform_index,
            ArchiveWorkPhase::RuntimeDelivery => &mut self.runtime_delivery,
            ArchiveWorkPhase::PresentationQuery => &mut self.presentation_queries,
        }
    }
}

fn active_sessions() -> &'static Mutex<HashMap<SourceIdentity, Weak<AttributionState>>> {
    static ACTIVE: OnceLock<Mutex<HashMap<SourceIdentity, Weak<AttributionState>>>> =
        OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn covered_bytes(ranges: &[Range<u64>], candidate: Range<u64>) -> u64 {
    ranges
        .iter()
        .map(|range| {
            range
                .end
                .min(candidate.end)
                .saturating_sub(range.start.max(candidate.start))
        })
        .sum()
}

fn insert_covered_range(ranges: &mut Vec<Range<u64>>, mut candidate: Range<u64>) {
    let mut index = 0;
    while index < ranges.len() {
        if ranges[index].end < candidate.start {
            index += 1;
        } else if ranges[index].start > candidate.end {
            break;
        } else {
            let range = ranges.remove(index);
            candidate.start = candidate.start.min(range.start);
            candidate.end = candidate.end.max(range.end);
        }
    }
    ranges.insert(index, candidate);
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod archive_work_attribution_tests {
    use super::*;

    #[test]
    fn source_overlap_is_attributed_to_the_phase_that_rereads_it() {
        let attribution = DslArchiveWorkAttribution::begin(SourceIdentity::from_bytes([0x31; 32]));
        let metadata = active_archive_work(
            attribution.snapshot().source_identity,
            ArchiveWorkPhase::Metadata,
        )
        .unwrap();
        let viewer = active_archive_work(
            attribution.snapshot().source_identity,
            ArchiveWorkPhase::PresentationQuery,
        )
        .unwrap();

        metadata.record_source_read(10, 20);
        viewer.record_source_read(20, 20);

        let profile = attribution.snapshot();
        assert_eq!(profile.metadata.source_reread_bytes, 0);
        assert_eq!(profile.presentation_queries.source_ranges_reread, 1);
        assert_eq!(profile.presentation_queries.source_reread_bytes, 10);
        assert_eq!(profile.total().source_bytes, 40);
    }
}
