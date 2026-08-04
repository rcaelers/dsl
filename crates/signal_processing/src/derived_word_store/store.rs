use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use web_time::Instant;

use signal_artifacts::{
    ArtifactKey, ArtifactRepository, ByteRange, RepositoryError, WriteArtifact,
    read_artifact_region,
};

use super::backend::{AnnotationStoreBackend, AnnotationStoreWriterBackend};
use super::cache::{cache_block, cached_block};
use super::codec::{
    DecodedWordBlock, EncodedBlockMetadata, WordBlockBuilder, decode_word_block,
    decode_word_block_range,
};
use super::config::{LiveStoreConfig, PersistentStoreConfig};
use super::errors::CodecError;
use super::format::{BLOCK_FLAG_HAS_DURATIONS, BlockDirectoryEntry, WordBlockHeader};
use super::persistent;
#[cfg(test)]
use super::presence::MAX_PRESENCE_RUNS_PER_BLOCK;
use super::presence::{WordPresenceIndex, WordSummaryRecord, word_presence_summaries};
use super::query::{
    AnnotationQuery, AnnotationQueryError, AnnotationQueryResult, AnnotationStoreMetadata,
    ExactAnnotationWindow, WordPresenceBucket, annotation_window_from_ordered_words,
    boundary_block_indices, exact_block_indices, nearest_boundary_from_ordered_words,
};
use super::state::{LiveStoreMetadata, LiveStoreSnapshot, StoreStatus};
use crate::WorkExecutor;
use crate::events::{Annotation, Word};

const MAX_BLOCK_ENCODERS_PER_STORE: usize = 4;
const TARGET_SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
const DERIVED_BLOCK_ENCODING_WORK: &str = "signal-processing.derived-block-encoding/v1";
static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);

fn block_encoder_count(available_workers: usize) -> usize {
    match available_workers {
        0..=2 => 1,
        workers => workers.div_ceil(4).clamp(2, MAX_BLOCK_ENCODERS_PER_STORE),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("derived-word artifact repository error: {0}")]
    Repository(#[from] RepositoryError),

    #[error("derived-word store codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("derived-word store is not live: {0:?}")]
    NotLive(StoreStatus),

    #[error("committed word block {index} is out of bounds (block count {block_count})")]
    BlockOutOfBounds { index: usize, block_count: usize },

    #[error("committed word-block directory does not match encoded block {0}")]
    DirectoryMismatch(u64),

    #[error("invalid persistent derived-word cache: {0}")]
    Persistent(String),
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

struct LiveState {
    directory: Vec<BlockDirectoryEntry>,
    presence: WordPresenceIndex,
    generation: u64,
    committed_word_count: u64,
    committed_data_len: u64,
    committed_first_timestamp_ns: Option<u64>,
    committed_last_timestamp_ns: Option<u64>,
    hot_tail: Arc<[Word]>,
    status: StoreStatus,
}

struct StoreShared {
    state: RwLock<LiveState>,
    repository: Arc<dyn ArtifactRepository>,
    store_identity: [u8; 32],
    store_id: u64,
    remove_on_drop: AtomicBool,
    persistent_cache: AtomicBool,
    pending_blocks: RwLock<BTreeMap<u64, Vec<u8>>>,
}

impl Drop for StoreShared {
    fn drop(&mut self) {
        if self.remove_on_drop.load(Ordering::Relaxed) {
            let directory = self.state.get_mut().unwrap().directory.clone();
            let mut previous_segment = None;
            for entry in directory {
                if previous_segment == Some(entry.segment_sequence) {
                    continue;
                }
                previous_segment = Some(entry.segment_sequence);
                if let Ok(key) = segment_key(self.store_identity, entry.segment_sequence) {
                    let _ = self.repository.remove(&key);
                }
            }
        }
    }
}

impl StoreShared {
    fn mark_failed(&self, message: String) {
        let mut state = self.state.write().unwrap();
        if matches!(state.status, StoreStatus::Live | StoreStatus::Finished) {
            state.status = StoreStatus::Failed(message);
            state.hot_tail = Arc::from([]);
            state.generation += 1;
        }
    }
}

/// Cloneable read handle for the committed prefix and current hot-tail snapshot.
#[derive(Clone)]
pub struct IndexedAnnotationStore {
    shared: Arc<StoreShared>,
}

impl IndexedAnnotationStore {
    /// Opens a finalized persistent store if all required artifacts are valid.
    ///
    /// # Parameters
    /// - `config`: Persistent cache identity and repository configuration.
    pub fn open_persistent(
        config: &PersistentStoreConfig,
    ) -> StoreResult<Option<IndexedAnnotationStore>> {
        let Some(index) = persistent::open(config)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            shared: Arc::new(StoreShared {
                state: RwLock::new(LiveState {
                    directory: index.directory,
                    presence: index.presence,
                    generation: 1,
                    committed_word_count: index.committed_word_count,
                    committed_data_len: index.committed_data_len,
                    committed_first_timestamp_ns: index.first_timestamp_ns,
                    committed_last_timestamp_ns: index.last_timestamp_ns,
                    hot_tail: Arc::from([]),
                    status: StoreStatus::Finished,
                }),
                repository: Arc::clone(&config.artifact_repository),
                store_identity: config.cache_key,
                store_id: NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed),
                remove_on_drop: AtomicBool::new(false),
                persistent_cache: AtomicBool::new(true),
                pending_blocks: RwLock::new(BTreeMap::new()),
            }),
        }))
    }

    /// Returns a consistent snapshot of committed data and the live hot tail.
    pub fn snapshot(&self) -> LiveStoreSnapshot {
        let state = self.shared.state.read().unwrap();
        let first_timestamp_ns = state
            .committed_first_timestamp_ns
            .or_else(|| state.hot_tail.first().map(|word| word.timestamp_ns));
        let last_timestamp_ns = state
            .hot_tail
            .last()
            .map(|word| word.timestamp_ns)
            .or(state.committed_last_timestamp_ns);
        let extent_end_ns = state
            .hot_tail
            .iter()
            .map(|word| word.timestamp_ns.saturating_add(word.duration_ns))
            .max()
            .into_iter()
            .chain(state.presence.extent_end_ns())
            .max();
        LiveStoreSnapshot {
            metadata: LiveStoreMetadata {
                generation: state.generation,
                committed_block_count: state.directory.len(),
                committed_word_count: state.committed_word_count,
                committed_data_len: state.committed_data_len,
                first_timestamp_ns,
                last_timestamp_ns,
                extent_end_ns,
                hot_tail_word_count: state.hot_tail.len(),
                immutable_region_backed: self.shared.repository.capabilities().immutable_regions,
                persistent_cache: self.shared.persistent_cache.load(Ordering::Relaxed),
                status: state.status.clone(),
            },
            hot_tail: Arc::clone(&state.hot_tail),
        }
    }

    #[cfg(test)]
    fn directory(&self) -> Vec<BlockDirectoryEntry> {
        self.shared.state.read().unwrap().directory.clone()
    }

    /// Visits each immutable committed block in timestamp order without
    /// cloning its decoded word vector. Intended for validation and export.
    pub fn visit_committed_blocks(
        &self,
        mut visitor: impl FnMut(CommittedAnnotationBlock<'_>),
    ) -> StoreResult<()> {
        let directory = self.shared.state.read().unwrap().directory.clone();
        for entry in directory {
            let block = self.read_cached_entry(entry)?;
            visitor(CommittedAnnotationBlock {
                restart_count: block.header.restart_count,
                words: &block.words,
            });
        }
        Ok(())
    }

    /// Fingerprints the exact immutable encoded word sequence without
    /// decoding or retaining complete blocks.
    ///
    /// Repository identity metadata is outside the committed blocks and therefore
    /// does not affect this content fingerprint. The store must be finished
    /// so an in-flight hot tail cannot be omitted.
    pub fn committed_data_fingerprint(&self) -> StoreResult<[u8; 32]> {
        let (directory, word_count) = {
            let state = self.shared.state.read().unwrap();
            if state.status != StoreStatus::Finished || !state.hot_tail.is_empty() {
                return Err(StoreError::Persistent(
                    "cannot fingerprint an unfinished derived-word store".to_owned(),
                ));
            }
            (state.directory.clone(), state.committed_word_count)
        };
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"logic-conduit-derived-words/v1");
        hasher.update(&word_count.to_le_bytes());
        hasher.update(&(directory.len() as u64).to_le_bytes());
        for entry in directory {
            let bytes = self.read_entry_bytes(entry)?;
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        Ok(*hasher.finalize().as_bytes())
    }

    /// Reads and validates one fully committed block. The directory lock is
    /// released before repository access or decoding occurs.
    #[cfg(test)]
    fn read_committed_block(&self, index: usize) -> StoreResult<DecodedWordBlock> {
        let entry = {
            let state = self.shared.state.read().unwrap();
            state
                .directory
                .get(index)
                .copied()
                .ok_or(StoreError::BlockOutOfBounds {
                    index,
                    block_count: state.directory.len(),
                })?
        };

        let result = self.read_cached_entry(entry).map(|block| (*block).clone());
        if let Err(error) = &result {
            self.shared.mark_failed(error.to_string());
        }
        result
    }

    fn read_cached_entry(&self, entry: BlockDirectoryEntry) -> StoreResult<Arc<DecodedWordBlock>> {
        if let Some(block) = cached_block(self.shared.store_id, entry.sequence) {
            return Ok(block);
        }
        let bytes = self.read_entry_bytes(entry)?;
        let decoded = decode_word_block(&bytes)?;
        validate_directory_header(decoded.header, entry)?;
        let decoded = Arc::new(decoded);
        cache_block(self.shared.store_id, Arc::clone(&decoded));
        Ok(decoded)
    }

    fn read_entry_bytes(&self, entry: BlockDirectoryEntry) -> StoreResult<Vec<u8>> {
        if let Some(bytes) = self
            .shared
            .pending_blocks
            .read()
            .unwrap()
            .get(&entry.sequence)
        {
            return Ok(bytes.clone());
        }
        let key = segment_key(self.shared.store_identity, entry.segment_sequence)?;
        let mut reader = self.shared.repository.open(&key)?.ok_or_else(|| {
            StoreError::Persistent(format!(
                "segment {} for committed word block {} is missing",
                entry.segment_sequence, entry.sequence
            ))
        })?;
        let end = entry
            .segment_offset
            .checked_add(u64::from(entry.block_len))
            .ok_or(StoreError::DirectoryMismatch(entry.sequence))?;
        if reader.len()? < end {
            return Err(StoreError::DirectoryMismatch(entry.sequence));
        }
        let range = ByteRange::new(entry.segment_offset, u64::from(entry.block_len))
            .map_err(RepositoryError::from)?;
        let region = read_artifact_region(&mut *reader, range)?;
        Ok(region.bytes().to_vec())
    }

    fn query_entry_words(
        &self,
        entry: BlockDirectoryEntry,
        start_ns: u64,
        end_ns: u64,
        max_context_words: usize,
    ) -> StoreResult<QueryBlockWords> {
        if let Some(block) = cached_block(self.shared.store_id, entry.sequence) {
            return Ok(QueryBlockWords::Cached(block));
        }
        if entry.flags & BLOCK_FLAG_HAS_DURATIONS as u8 != 0
            || start_ns <= entry.first_timestamp_ns && end_ns >= entry.last_timestamp_ns
        {
            return self.read_cached_entry(entry).map(QueryBlockWords::Cached);
        }

        let bytes = self.read_entry_bytes(entry)?;
        let range = decode_word_block_range(&bytes, start_ns, end_ns, max_context_words)?;
        validate_directory_header(range.header, entry)?;
        Ok(QueryBlockWords::Partial {
            words: range.words,
            complete: range.complete,
        })
    }

    fn exact_context(
        &self,
        start_ns: u64,
        end_ns: u64,
    ) -> (u64, Vec<BlockDirectoryEntry>, Arc<[Word]>) {
        let state = self.shared.state.read().unwrap();
        let entries = exact_block_indices(&state.directory, &state.presence, start_ns, end_ns)
            .into_iter()
            .map(|index| state.directory[index])
            .collect();
        (state.generation, entries, Arc::clone(&state.hot_tail))
    }

    fn boundary_context(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> (Vec<BlockDirectoryEntry>, Arc<[Word]>) {
        let state = self.shared.state.read().unwrap();
        let entries = boundary_block_indices(
            &state.directory,
            &state.presence,
            timestamp_ns,
            max_distance_ns,
        )
        .into_iter()
        .map(|index| state.directory[index])
        .collect();
        (entries, Arc::clone(&state.hot_tail))
    }
}

/// Borrowed, format-independent view of one committed annotation block.
#[derive(Clone, Copy, Debug)]
pub struct CommittedAnnotationBlock<'a> {
    /// Number of internal seek restart points in the block.
    pub restart_count: u32,
    /// Decoded annotations in timestamp order.
    pub words: &'a [Word],
}

impl AnnotationQuery for IndexedAnnotationStore {
    fn metadata(&self) -> AnnotationStoreMetadata {
        let snapshot = self.snapshot();
        AnnotationStoreMetadata {
            generation: snapshot.metadata.generation,
            is_live: snapshot.metadata.status == StoreStatus::Live,
            total_word_count: snapshot.metadata.committed_word_count
                + snapshot.metadata.hot_tail_word_count as u64,
            first_timestamp_ns: snapshot.metadata.first_timestamp_ns,
            last_timestamp_ns: snapshot.metadata.last_timestamp_ns,
            extent_end_ns: snapshot.metadata.extent_end_ns,
        }
    }

    fn presence_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        target_buckets: usize,
    ) -> AnnotationQueryResult<Vec<WordPresenceBucket>> {
        let mut buckets = self.coarse_presence_window(start_ns, end_ns, target_buckets)?;
        // Exact boxes switch to presence at two words per pixel. Spend a
        // bounded amount of additional decode work here so moderately dense
        // burst traffic still preserves visible gaps instead of smearing one
        // block summary across the whole viewport. The coarse estimate keeps
        // truly dense overviews on the no-decode mipmap path.
        let refine_limit = target_buckets.saturating_mul(8).max(32);
        let estimated_words = buckets
            .iter()
            .map(|bucket| bucket.word_count)
            .fold(0u64, u64::saturating_add);
        if estimated_words <= refine_limit as u64
            && let Ok(exact) = self.exact_window(start_ns, end_ns, refine_limit)
            && exact.complete
        {
            return Ok(annotation_presence_buckets(
                &exact.annotations,
                start_ns,
                end_ns,
                target_buckets,
            ));
        }
        buckets.retain(|bucket| bucket.word_count > 0);
        Ok(buckets)
    }

    fn coarse_presence_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        target_buckets: usize,
    ) -> AnnotationQueryResult<Vec<WordPresenceBucket>> {
        if start_ns > end_ns {
            return Err(AnnotationQueryError::InvalidWindow { start_ns, end_ns });
        }
        if target_buckets == 0 {
            return Err(AnnotationQueryError::ZeroBucketLimit);
        }
        let mut buckets = {
            let state = self.shared.state.read().unwrap();
            let mut buckets = state
                .presence
                .presence_window_all(start_ns, end_ns, target_buckets);
            merge_hot_tail_presence(&mut buckets, &state.hot_tail);
            buckets
        };
        buckets.retain(|bucket| bucket.word_count > 0);
        Ok(buckets)
    }

    fn exact_window(
        &self,
        start_ns: u64,
        end_ns: u64,
        max_words: usize,
    ) -> AnnotationQueryResult<ExactAnnotationWindow> {
        if start_ns > end_ns {
            return Err(AnnotationQueryError::InvalidWindow { start_ns, end_ns });
        }
        if max_words == 0 {
            return Err(AnnotationQueryError::ZeroWordLimit);
        }
        let (generation, entries, hot_tail) = self.exact_context(start_ns, end_ns);
        let mut candidates = ExactQueryCandidates::new(max_words);

        for entry in entries {
            let remaining = max_words
                .saturating_sub(candidates.words.len())
                .saturating_add(3)
                .max(3);
            let block = self
                .query_entry_words(entry, start_ns, end_ns, remaining)
                .map_err(query_store_error)?;
            candidates.collect(block.words(), start_ns, end_ns);
            if !block.complete() {
                candidates.truncated = true;
            }
            if candidates.truncated || candidates.successor.is_some() {
                break;
            }
        }
        if !candidates.truncated && candidates.successor.is_none() {
            candidates.collect(&hot_tail, start_ns, end_ns);
        }

        let mut context = candidates.words;
        if let Some(previous) = candidates.previous_predecessor
            && !context.contains(&previous)
        {
            context.push(previous);
        }
        if let Some(predecessor) = candidates.predecessor
            && !context.contains(&predecessor)
        {
            context.push(predecessor);
        }
        if let Some(successor) = candidates.successor {
            context.push(successor);
        }
        context.sort_by_key(|word| word.timestamp_ns);

        let (annotations, truncated) =
            annotation_window_from_ordered_words(&context, start_ns, end_ns, max_words);
        candidates.truncated |= truncated;

        Ok(ExactAnnotationWindow {
            annotations,
            complete: !candidates.truncated,
            generation,
        })
    }

    fn latest_word_at_or_before(&self, timestamp_ns: u64) -> AnnotationQueryResult<Option<Word>> {
        let (_, entries, hot_tail) = self.exact_context(timestamp_ns, timestamp_ns);
        let mut latest = hot_tail
            .iter()
            .filter(|word| word.timestamp_ns <= timestamp_ns)
            .max_by_key(|word| word.timestamp_ns)
            .cloned();
        for entry in entries {
            let block = self
                .query_entry_words(entry, timestamp_ns, timestamp_ns, 3)
                .map_err(query_store_error)?;
            if let Some(word) = block
                .words()
                .iter()
                .filter(|word| word.timestamp_ns <= timestamp_ns)
                .max_by_key(|word| word.timestamp_ns)
                && latest
                    .as_ref()
                    .is_none_or(|current| current.timestamp_ns <= word.timestamp_ns)
            {
                latest = Some(word.clone());
            }
        }
        Ok(latest)
    }

    fn nearest_boundary(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> AnnotationQueryResult<Option<u64>> {
        let (entries, hot_tail) = self.boundary_context(timestamp_ns, max_distance_ns);
        let lower = timestamp_ns.saturating_sub(max_distance_ns);
        let upper = timestamp_ns.saturating_add(max_distance_ns);
        let mut context = Vec::new();

        for entry in entries {
            let block = self
                .query_entry_words(entry, lower, upper, entry.word_count as usize + 2)
                .map_err(query_store_error)?;
            context.extend_from_slice(block.words());
        }
        context.extend_from_slice(&hot_tail);
        context.sort_by_key(|word| word.timestamp_ns);
        context.dedup();
        Ok(nearest_boundary_from_ordered_words(
            &context,
            timestamp_ns,
            max_distance_ns,
        ))
    }
}

fn annotation_presence_buckets(
    annotations: &[Annotation],
    start_ns: u64,
    end_ns: u64,
    target_buckets: usize,
) -> Vec<WordPresenceBucket> {
    if annotations.is_empty() || target_buckets == 0 || start_ns > end_ns {
        return Vec::new();
    }
    let span = end_ns.saturating_sub(start_ns).saturating_add(1);
    let bucket_count = target_buckets
        .min(usize::try_from(span).unwrap_or(usize::MAX))
        .max(1);
    let mut starts = vec![0u64; bucket_count];
    let mut ends = vec![0u64; bucket_count + 1];
    for annotation in annotations {
        let overlap_start = annotation.start_ns.max(start_ns);
        let overlap_end = annotation.end_ns.min(end_ns);
        if overlap_start > overlap_end {
            continue;
        }
        let first = ((u128::from(overlap_start - start_ns) * bucket_count as u128)
            / u128::from(span)) as usize;
        let last = ((u128::from(overlap_end - start_ns) * bucket_count as u128) / u128::from(span))
            as usize;
        let first = first.min(bucket_count - 1);
        let last = last.min(bucket_count - 1);
        starts[first] = starts[first].saturating_add(1);
        ends[last + 1] = ends[last + 1].saturating_add(1);
    }

    let mut running_count = 0u64;
    starts
        .into_iter()
        .enumerate()
        .filter_map(|(index, starting)| {
            running_count = running_count.saturating_sub(ends[index]);
            running_count = running_count.saturating_add(starting);
            let word_count = running_count;
            if word_count == 0 {
                return None;
            }
            let bucket_start = start_ns
                .saturating_add(((u128::from(span) * index as u128) / bucket_count as u128) as u64);
            let bucket_end_exclusive = start_ns.saturating_add(
                ((u128::from(span) * (index + 1) as u128) / bucket_count as u128) as u64,
            );
            Some(WordPresenceBucket {
                start_ns: bucket_start,
                end_ns: bucket_end_exclusive
                    .max(bucket_start.saturating_add(1))
                    .saturating_sub(1)
                    .min(end_ns),
                word_count,
            })
        })
        .collect()
}

enum QueryBlockWords {
    Cached(Arc<DecodedWordBlock>),
    Partial { words: Vec<Word>, complete: bool },
}

fn merge_hot_tail_presence(buckets: &mut [WordPresenceBucket], words: &[Word]) {
    for word in words {
        let word_end = word.timestamp_ns.saturating_add(word.duration_ns);
        let first = buckets.partition_point(|bucket| bucket.end_ns < word.timestamp_ns);
        let end = buckets.partition_point(|bucket| bucket.start_ns <= word_end);
        for bucket in &mut buckets[first.min(end)..end] {
            bucket.word_count = bucket.word_count.saturating_add(1);
        }
    }
}

impl QueryBlockWords {
    fn words(&self) -> &[Word] {
        match self {
            Self::Cached(block) => &block.words,
            Self::Partial { words, .. } => words,
        }
    }

    fn complete(&self) -> bool {
        match self {
            Self::Cached(_) => true,
            Self::Partial { complete, .. } => *complete,
        }
    }
}

fn validate_directory_header(
    header: WordBlockHeader,
    entry: BlockDirectoryEntry,
) -> StoreResult<()> {
    if header.sequence != entry.sequence
        || header.first_timestamp_ns != entry.first_timestamp_ns
        || header.last_timestamp_ns != entry.last_timestamp_ns
        || header.block_len != entry.block_len
        || header.word_count != entry.word_count
        || header.value_bytes != entry.value_bytes
        || header.flags as u8 != entry.flags
    {
        return Err(StoreError::DirectoryMismatch(entry.sequence));
    }
    Ok(())
}

struct ExactQueryCandidates {
    words: Vec<Word>,
    previous_predecessor: Option<Word>,
    predecessor: Option<Word>,
    successor: Option<Word>,
    truncated: bool,
    limit: usize,
}

impl ExactQueryCandidates {
    fn new(limit: usize) -> Self {
        Self {
            words: Vec::with_capacity(limit),
            previous_predecessor: None,
            predecessor: None,
            successor: None,
            truncated: false,
            limit,
        }
    }

    fn collect(&mut self, words: &[Word], start_ns: u64, end_ns: u64) {
        if words.is_empty() || self.truncated || self.successor.is_some() {
            return;
        }
        for word in words {
            if word.timestamp_ns < start_ns {
                if self
                    .predecessor
                    .as_ref()
                    .is_none_or(|current| current.timestamp_ns <= word.timestamp_ns)
                {
                    self.previous_predecessor = self.predecessor.clone();
                    self.predecessor = Some(word.clone());
                }
                if word.duration_ns == 0
                    || word.timestamp_ns.saturating_add(word.duration_ns) < start_ns
                {
                    continue;
                }
            } else if word.timestamp_ns > end_ns {
                self.successor = Some(word.clone());
                break;
            }

            if self.words.len() == self.limit {
                self.truncated = true;
                return;
            }
            self.words.push(word.clone());
        }
    }
}

fn query_store_error(error: StoreError) -> AnnotationQueryError {
    AnnotationQueryError::Store(error.to_string())
}

struct PreparedBlock {
    builder: WordBlockBuilder,
    encoded: Vec<u8>,
    result: Result<(EncodedBlockMetadata, Vec<WordSummaryRecord>), CodecError>,
}

struct BlockCompletion {
    sequence: u64,
    block: Option<PreparedBlock>,
}

struct ActiveSegment {
    sequence: u64,
    writer: Box<dyn WriteArtifact>,
    length: u64,
    block_sequences: Vec<u64>,
}

fn prepare_encoded_block(
    sequence: u64,
    mut builder: WordBlockBuilder,
    mut encoded: Vec<u8>,
) -> PreparedBlock {
    let duration_free = builder.is_duration_free();
    let summaries = word_presence_summaries(sequence, builder.words(), duration_free);
    let result = builder
        .encode(sequence, &mut encoded)
        .map(|metadata| (metadata, summaries));
    if result.is_err() {
        encoded.clear();
    }
    builder.clear();
    PreparedBlock {
        builder,
        encoded,
        result,
    }
}

/// Single-threaded append side of a live indexed annotation store.
pub struct IndexedAnnotationWriter {
    shared: Arc<StoreShared>,
    builder: WordBlockBuilder,
    completion_sender: Sender<BlockCompletion>,
    completion_receiver: Receiver<BlockCompletion>,
    prepared_blocks: BTreeMap<u64, PreparedBlock>,
    available_builders: Vec<WordBlockBuilder>,
    available_encoded_blocks: Vec<Vec<u8>>,
    in_flight_blocks: usize,
    max_outstanding_blocks: usize,
    next_dispatch_sequence: u64,
    next_sequence: u64,
    next_data_offset: u64,
    last_timestamp_ns: Option<u64>,
    words_since_tail_publish: usize,
    last_tail_publish: Instant,
    hot_tail_publish_words: usize,
    hot_tail_publish_interval: Duration,
    terminal: bool,
    persistence: Option<PersistentStoreConfig>,
    work_executor: Arc<dyn WorkExecutor>,
    created_unix_ns: u64,
    active_segment: Option<ActiveSegment>,
    next_segment_sequence: u64,
    target_segment_bytes: u64,
}

impl IndexedAnnotationWriter {
    /// Creates a live writer together with its cloneable query handle.
    ///
    /// # Parameters
    /// - `config`: Live encoding, publishing, execution, and persistence policy.
    pub fn create(config: LiveStoreConfig) -> StoreResult<(Self, IndexedAnnotationStore)> {
        if config.hot_tail_publish_words == 0 {
            return Err(StoreError::Codec(CodecError::InvalidConfiguration(
                "hot_tail_publish_words must be greater than zero",
            )));
        }
        let created_unix_ns = config
            .persistence
            .as_ref()
            .map_or(0, |persistent| persistent.time_source.now_unix_ns());
        let store_id = NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed);
        let store_identity = config.persistence.as_ref().map_or_else(
            || ephemeral_store_identity(config.cache_key_prefix, store_id),
            |persistent| persistent.cache_key,
        );
        if let Some(persistent) = &config.persistence {
            persistent::invalidate(persistent)?;
        }
        let repository = config.persistence.as_ref().map_or_else(
            || Arc::clone(&config.artifact_repository),
            |persistent| Arc::clone(&persistent.artifact_repository),
        );
        let builder = WordBlockBuilder::new(config.block)?;
        let work_executor = Arc::clone(&config.work_executor);
        let max_outstanding_blocks = block_encoder_count(work_executor.available_parallelism());
        let (completion_sender, completion_receiver) = bounded(max_outstanding_blocks);
        let now = Instant::now();
        let last_tail_publish = now
            .checked_sub(config.hot_tail_publish_interval)
            .unwrap_or(now);
        let shared = Arc::new(StoreShared {
            state: RwLock::new(LiveState {
                directory: Vec::new(),
                presence: WordPresenceIndex::new(),
                generation: 0,
                committed_word_count: 0,
                committed_data_len: 0,
                committed_first_timestamp_ns: None,
                committed_last_timestamp_ns: None,
                hot_tail: Arc::from([]),
                status: StoreStatus::Live,
            }),
            repository,
            store_identity,
            store_id,
            remove_on_drop: AtomicBool::new(true),
            persistent_cache: AtomicBool::new(false),
            pending_blocks: RwLock::new(BTreeMap::new()),
        });
        let store = IndexedAnnotationStore {
            shared: Arc::clone(&shared),
        };
        Ok((
            Self {
                shared,
                builder,
                completion_sender,
                completion_receiver,
                prepared_blocks: BTreeMap::new(),
                available_builders: Vec::new(),
                available_encoded_blocks: Vec::new(),
                in_flight_blocks: 0,
                max_outstanding_blocks,
                next_dispatch_sequence: 0,
                next_sequence: 0,
                next_data_offset: 0,
                last_timestamp_ns: None,
                words_since_tail_publish: 0,
                last_tail_publish,
                hot_tail_publish_words: config.hot_tail_publish_words,
                hot_tail_publish_interval: config.hot_tail_publish_interval,
                terminal: false,
                persistence: config.persistence,
                work_executor,
                created_unix_ns,
                active_segment: None,
                next_segment_sequence: 0,
                target_segment_bytes: TARGET_SEGMENT_BYTES,
            },
            store,
        ))
    }

    /// Returns a cloneable query handle for the writer's current store.
    pub fn store(&self) -> IndexedAnnotationStore {
        IndexedAnnotationStore {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Appends one timestamp-ordered word to the live store.
    ///
    /// # Parameters
    /// - `word`: Next word, whose timestamp must not precede prior appends.
    pub fn append(&mut self, word: Word) -> StoreResult<()> {
        self.append_batch(std::slice::from_ref(&word))
    }

    /// Appends a timestamp-ordered batch and publishes ready immutable blocks.
    ///
    /// # Parameters
    ///
    /// - `words`: Batch whose timestamps do not precede earlier appends.
    pub fn append_batch(&mut self, words: &[Word]) -> StoreResult<()> {
        self.ensure_live()?;
        let result = self
            .append_batch_inner(words)
            .and_then(|()| self.publish_appended_prefix());
        if let Err(error) = &result {
            self.fail(error);
        }
        result
    }

    /// Publishes the current mutable tail for live query consumers.
    pub fn publish_hot_tail(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        self.flush_dispatched_blocks()?;
        self.publish_hot_tail_inner();
        Ok(())
    }

    /// Flushes all blocks and publishes a final immutable store manifest.
    pub fn finish(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        let result = self.finish_inner();
        if let Err(error) = &result {
            self.fail(error);
        }
        result
    }

    /// Cancels without publishing a manifest. Unfinished block artifacts are
    /// removed when the final writer/query handle is dropped.
    pub fn cancel(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        self.builder.clear();
        self.words_since_tail_publish = 0;
        self.discard_active_segment();
        let mut state = self.shared.state.write().unwrap();
        state.hot_tail = Arc::from([]);
        state.status = StoreStatus::Cancelled;
        state.generation += 1;
        self.terminal = true;
        Ok(())
    }

    fn append_batch_inner(&mut self, words: &[Word]) -> StoreResult<()> {
        self.drain_completed_blocks()?;
        for word in words {
            if let Some(previous_timestamp_ns) = self.last_timestamp_ns
                && word.timestamp_ns < previous_timestamp_ns
            {
                return Err(StoreError::Codec(CodecError::OutOfOrder {
                    index: self.builder.len(),
                    previous_timestamp_ns,
                    timestamp_ns: word.timestamp_ns,
                }));
            }
            self.last_timestamp_ns = Some(word.timestamp_ns);
        }

        let mut remaining = words;
        while !remaining.is_empty() {
            let accepted = self.builder.extend_ordered(remaining);
            self.words_since_tail_publish += accepted;
            remaining = &remaining[accepted..];
            if !remaining.is_empty() || self.builder.is_at_word_limit() {
                self.dispatch_current_block()?;
            }
        }

        self.drain_completed_blocks()?;
        Ok(())
    }

    fn publish_appended_prefix(&mut self) -> StoreResult<()> {
        // Keep encoding asynchronous between collector drains. Backpressure in
        // `dispatch_current_block` bounds the queued builders, while completed
        // blocks are still published in sequence on every subsequent append.
        self.drain_completed_blocks()?;
        if self.outstanding_blocks() == 0
            && !self.builder.is_empty()
            && (self.words_since_tail_publish >= self.hot_tail_publish_words
                || self.last_tail_publish.elapsed() >= self.hot_tail_publish_interval)
        {
            self.publish_hot_tail_inner();
        }
        Ok(())
    }

    fn append_batches_inner(&mut self, batches: &[Vec<Word>]) -> StoreResult<()> {
        batches
            .iter()
            .try_for_each(|batch| self.append_batch_inner(batch))?;
        self.publish_appended_prefix()
    }

    fn finish_inner(&mut self) -> StoreResult<()> {
        self.dispatch_current_block()?;
        self.flush_dispatched_blocks()?;
        self.publish_active_segment()?;
        if let Some(persistent) = self.persistence.clone() {
            {
                let state = self.shared.state.read().unwrap();
                persistent::publish(
                    &persistent,
                    persistent::Publication {
                        directory: &state.directory,
                        presence: &state.presence,
                        committed_word_count: state.committed_word_count,
                        committed_data_len: state.committed_data_len,
                        first_timestamp_ns: state.committed_first_timestamp_ns,
                        last_timestamp_ns: state.committed_last_timestamp_ns,
                        created_unix_ns: self.created_unix_ns,
                    },
                )?;
            }
            self.shared.remove_on_drop.store(false, Ordering::Relaxed);
            self.shared.persistent_cache.store(true, Ordering::Relaxed);
        }
        let mut state = self.shared.state.write().unwrap();
        state.status = StoreStatus::Finished;
        state.generation += 1;
        self.terminal = true;
        Ok(())
    }

    fn outstanding_blocks(&self) -> usize {
        self.in_flight_blocks + self.prepared_blocks.len()
    }

    fn dispatch_current_block(&mut self) -> StoreResult<()> {
        if self.builder.is_empty() {
            return Ok(());
        }
        self.drain_completed_blocks()?;
        while self.outstanding_blocks() >= self.max_outstanding_blocks {
            self.receive_completed_block()?;
            self.commit_ordered_blocks()?;
        }

        let replacement = self
            .available_builders
            .pop()
            .unwrap_or_else(|| self.builder.empty_like());
        let builder = std::mem::replace(&mut self.builder, replacement);
        let encoded = self.available_encoded_blocks.pop().unwrap_or_default();
        let sequence = self.next_dispatch_sequence;
        let completion = self.completion_sender.clone();
        self.work_executor
            .submit_labeled(
                DERIVED_BLOCK_ENCODING_WORK,
                Box::new(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        prepare_encoded_block(sequence, builder, encoded)
                    }));
                    let message = BlockCompletion {
                        sequence,
                        block: result.ok(),
                    };
                    let _ = completion.send(message);
                }),
            )
            .map_err(StoreError::Persistent)?;
        self.in_flight_blocks += 1;
        self.next_dispatch_sequence += 1;
        self.words_since_tail_publish = 0;
        Ok(())
    }

    fn drain_completed_blocks(&mut self) -> StoreResult<()> {
        loop {
            match self.completion_receiver.try_recv() {
                Ok(completion) => self.accept_completed_block(completion)?,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(StoreError::Persistent(
                        "block encoder completion channel closed".into(),
                    ));
                }
            }
        }
        self.commit_ordered_blocks()
    }

    fn receive_completed_block(&mut self) -> StoreResult<()> {
        let completion = self.completion_receiver.recv().map_err(|_| {
            StoreError::Persistent("block encoder completion channel closed".into())
        })?;
        self.accept_completed_block(completion)
    }

    fn accept_completed_block(&mut self, completion: BlockCompletion) -> StoreResult<()> {
        self.in_flight_blocks = self.in_flight_blocks.checked_sub(1).ok_or_else(|| {
            StoreError::Persistent("received an unexpected block encoder completion".into())
        })?;
        let BlockCompletion { sequence, block } = completion;
        let Some(block) = block else {
            return Err(StoreError::Persistent(format!(
                "block encoder panicked for sequence {sequence}"
            )));
        };
        if self.prepared_blocks.insert(sequence, block).is_some() {
            return Err(StoreError::Persistent(format!(
                "block encoder completed sequence {sequence} twice"
            )));
        }
        Ok(())
    }

    fn commit_ordered_blocks(&mut self) -> StoreResult<()> {
        while let Some(block) = self.prepared_blocks.remove(&self.next_sequence) {
            self.commit_prepared_block(block)?;
        }
        Ok(())
    }

    fn flush_dispatched_blocks(&mut self) -> StoreResult<()> {
        while self.outstanding_blocks() > 0 {
            if self.prepared_blocks.contains_key(&self.next_sequence) {
                self.commit_ordered_blocks()?;
            } else {
                self.receive_completed_block()?;
                self.commit_ordered_blocks()?;
            }
        }
        Ok(())
    }

    fn commit_prepared_block(&mut self, mut block: PreparedBlock) -> StoreResult<()> {
        let (metadata, summaries) = block.result?;
        let header = metadata.header;
        debug_assert_eq!(header.sequence, self.next_sequence);
        self.prepare_active_segment(block.encoded.len())?;
        let active = self
            .active_segment
            .as_mut()
            .expect("preparing a non-empty block creates an active segment");
        let segment_sequence = active.sequence;
        let segment_offset = active.length;
        active.writer.write_at(segment_offset, &block.encoded)?;
        active.length = active
            .length
            .checked_add(block.encoded.len() as u64)
            .ok_or_else(|| StoreError::Persistent("derived segment length overflow".into()))?;
        active.block_sequences.push(header.sequence);
        let entry = BlockDirectoryEntry {
            sequence: header.sequence,
            first_timestamp_ns: header.first_timestamp_ns,
            last_timestamp_ns: header.last_timestamp_ns,
            data_offset: self.next_data_offset,
            segment_sequence,
            segment_offset,
            block_len: header.block_len,
            word_count: header.word_count,
            value_bytes: header.value_bytes,
            flags: header.flags as u8,
        };
        self.shared
            .pending_blocks
            .write()
            .unwrap()
            .insert(entry.sequence, std::mem::take(&mut block.encoded));
        {
            let mut state = self.shared.state.write().unwrap();
            state.directory.push(entry);
            for summary in summaries {
                state.presence.push(summary);
            }
            state.committed_word_count += u64::from(entry.word_count);
            state.committed_data_len = entry.data_offset + u64::from(entry.block_len);
            state
                .committed_first_timestamp_ns
                .get_or_insert(entry.first_timestamp_ns);
            state.committed_last_timestamp_ns = Some(entry.last_timestamp_ns);
            state.hot_tail = Arc::from([]);
            state.generation += 1;
        }

        self.next_sequence += 1;
        self.next_data_offset += u64::from(entry.block_len);
        self.available_builders.push(block.builder);
        self.last_tail_publish = Instant::now();
        Ok(())
    }

    fn prepare_active_segment(&mut self, block_len: usize) -> StoreResult<()> {
        let block_len = u64::try_from(block_len)
            .map_err(|_| StoreError::Persistent("derived block length exceeds u64".into()))?;
        if self.active_segment.as_ref().is_some_and(|segment| {
            segment.length > 0
                && segment.length.saturating_add(block_len) > self.target_segment_bytes
        }) {
            self.publish_active_segment()?;
        }
        if self.active_segment.is_none() {
            let sequence = self.next_segment_sequence;
            let writer = self
                .shared
                .repository
                .begin_write(segment_key(self.shared.store_identity, sequence)?)?;
            self.active_segment = Some(ActiveSegment {
                sequence,
                writer,
                length: 0,
                block_sequences: Vec::new(),
            });
            self.next_segment_sequence = self.next_segment_sequence.saturating_add(1);
        }
        Ok(())
    }

    fn publish_active_segment(&mut self) -> StoreResult<()> {
        let Some(mut segment) = self.active_segment.take() else {
            return Ok(());
        };
        segment.writer.truncate(segment.length)?;
        // Segments are reconstructable until the flushed index and manifest
        // expose the complete generation, so publication omits a per-segment
        // durability barrier just as the previous block artifacts did.
        segment.writer.publish()?;
        let mut pending = self.shared.pending_blocks.write().unwrap();
        for sequence in segment.block_sequences {
            if let Some(mut encoded) = pending.remove(&sequence) {
                encoded.clear();
                self.available_encoded_blocks.push(encoded);
            }
        }
        Ok(())
    }

    fn discard_active_segment(&mut self) {
        let Some(segment) = self.active_segment.take() else {
            return;
        };
        let mut pending = self.shared.pending_blocks.write().unwrap();
        for sequence in segment.block_sequences {
            if let Some(mut encoded) = pending.remove(&sequence) {
                encoded.clear();
                self.available_encoded_blocks.push(encoded);
            }
        }
    }

    fn publish_hot_tail_inner(&mut self) {
        let snapshot: Arc<[Word]> = Arc::from(self.builder.words().to_vec());
        let mut state = self.shared.state.write().unwrap();
        state.hot_tail = snapshot;
        state.generation += 1;
        self.words_since_tail_publish = 0;
        self.last_tail_publish = Instant::now();
    }

    fn ensure_live(&self) -> StoreResult<()> {
        let status = self.shared.state.read().unwrap().status.clone();
        if status == StoreStatus::Live {
            Ok(())
        } else {
            Err(StoreError::NotLive(status))
        }
    }

    fn fail(&mut self, error: &StoreError) {
        self.shared.mark_failed(error.to_string());
        self.terminal = true;
    }
}

impl AnnotationStoreBackend for IndexedAnnotationStore {
    fn snapshot(&self) -> LiveStoreSnapshot {
        IndexedAnnotationStore::snapshot(self)
    }
}

impl AnnotationStoreWriterBackend for IndexedAnnotationWriter {
    fn append_batch(&mut self, words: &[Word]) -> StoreResult<()> {
        IndexedAnnotationWriter::append_batch(self, words)
    }

    fn append_batches(&mut self, batches: &[Vec<Word>]) -> StoreResult<()> {
        self.ensure_live()?;
        let result = self.append_batches_inner(batches);
        if let Err(error) = &result {
            self.fail(error);
        }
        result
    }

    fn finish(&mut self) -> StoreResult<()> {
        IndexedAnnotationWriter::finish(self)
    }
}

impl Drop for IndexedAnnotationWriter {
    fn drop(&mut self) {
        if self.terminal {
            return;
        }
        self.discard_active_segment();
        let mut state = self.shared.state.write().unwrap();
        if state.status == StoreStatus::Live {
            state.hot_tail = Arc::from([]);
            state.status = StoreStatus::Cancelled;
            state.generation += 1;
        }
    }
}

fn segment_key(store_identity: [u8; 32], sequence: u64) -> StoreResult<ArtifactKey> {
    persistent::segment_key(store_identity, sequence)
}

fn ephemeral_store_identity(cache_key_prefix: [u8; 16], store_id: u64) -> [u8; 32] {
    let mut identity = [0_u8; 32];
    identity[..16].copy_from_slice(&cache_key_prefix);
    identity[16..24].copy_from_slice(&store_id.to_le_bytes());
    identity[24..].copy_from_slice(&(!store_id).to_le_bytes());
    identity
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;

    use signal_artifacts::MemoryArtifactRepository;

    use super::*;
    use crate::derived_word_store::BlockCodecConfig;
    use crate::derived_word_store::cache::cache_contains;
    use crate::events::instantaneous_word_end_ns;

    struct RecordingWorkExecutor {
        workers: usize,
        submissions: AtomicUsize,
    }

    struct QueuedWorkExecutor {
        tasks: Mutex<Vec<crate::WorkExecutorTask>>,
    }

    impl QueuedWorkExecutor {
        fn new() -> Self {
            Self {
                tasks: Mutex::new(Vec::new()),
            }
        }

        fn run_pending(&self) {
            let tasks = std::mem::take(&mut *self.tasks.lock().unwrap());
            tasks.into_iter().for_each(|task| task());
        }
    }

    impl WorkExecutor for QueuedWorkExecutor {
        fn available_parallelism(&self) -> usize {
            8
        }

        fn submit(
            &self,
            task: crate::WorkExecutorTask,
        ) -> Result<Box<dyn crate::WorkTask>, String> {
            self.tasks.lock().unwrap().push(task);
            Ok(Box::new(crate::CompletedWorkTask))
        }
    }

    impl RecordingWorkExecutor {
        fn new(workers: usize) -> Self {
            Self {
                workers,
                submissions: AtomicUsize::new(0),
            }
        }
    }

    impl WorkExecutor for RecordingWorkExecutor {
        fn available_parallelism(&self) -> usize {
            self.workers
        }

        fn submit(
            &self,
            task: crate::WorkExecutorTask,
        ) -> Result<Box<dyn crate::WorkTask>, String> {
            self.submissions.fetch_add(1, Ordering::Relaxed);
            task();
            Ok(Box::new(crate::CompletedWorkTask))
        }
    }

    fn test_config() -> LiveStoreConfig {
        LiveStoreConfig {
            block: BlockCodecConfig {
                max_words: 16,
                ..BlockCodecConfig::default()
            },
            hot_tail_publish_words: 4,
            hot_tail_publish_interval: Duration::from_millis(10),
            ..LiveStoreConfig::default()
        }
    }

    fn persistent_config(cache_key: [u8; 32]) -> LiveStoreConfig {
        LiveStoreConfig {
            persistence: Some(PersistentStoreConfig::new(cache_key)),
            ..test_config()
        }
    }

    #[test]
    fn block_encoding_parallelism_reserves_capacity_for_other_work() {
        assert_eq!(block_encoder_count(0), 1);
        assert_eq!(block_encoder_count(1), 1);
        assert_eq!(block_encoder_count(2), 1);
        assert_eq!(block_encoder_count(3), 2);
        assert_eq!(block_encoder_count(8), 2);
        assert_eq!(block_encoder_count(9), 3);
        assert_eq!(block_encoder_count(16), MAX_BLOCK_ENCODERS_PER_STORE);
        assert_eq!(
            block_encoder_count(usize::MAX),
            MAX_BLOCK_ENCODERS_PER_STORE
        );
    }

    #[test]
    fn configured_executor_bounds_and_receives_block_encoding_work() {
        let executor = Arc::new(RecordingWorkExecutor::new(9));
        let config = test_config().with_work_executor(executor.clone());
        let (mut writer, _) = IndexedAnnotationWriter::create(config).unwrap();

        let words = (0..32)
            .map(|index| Word::new(index, index * 10))
            .collect::<Vec<_>>();
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        assert_eq!(writer.max_outstanding_blocks, block_encoder_count(9));
        assert_eq!(executor.submissions.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn append_does_not_wait_for_dispatched_block_encoding() {
        let executor = Arc::new(QueuedWorkExecutor::new());
        let config = test_config().with_work_executor(executor.clone());
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let words = (0..16)
            .map(|index| Word::new(index, index * 10))
            .collect::<Vec<_>>();

        writer.append_batch(&words).unwrap();

        assert_eq!(store.snapshot().metadata.committed_word_count, 0);
        assert_eq!(executor.tasks.lock().unwrap().len(), 1);

        executor.run_pending();
        writer.append_batch(&[]).unwrap();

        assert_eq!(store.snapshot().metadata.committed_word_count, 16);
        writer.finish().unwrap();
    }

    #[test]
    fn persistent_finish_reopens_exact_words_and_presence_from_manifest() {
        let cache_key = [0x5a; 32];
        let config = persistent_config(cache_key);
        let persistent = config.persistence.clone().unwrap();
        let (mut writer, live_store) = IndexedAnnotationWriter::create(config).unwrap();
        let words: Vec<_> = (0..41)
            .map(|index| match index {
                17 => Word::bytes_with_tag(index, vec![0xa5; 96], index * 80, index % 5),
                23 => Word::labeled(index, "decoded label", index * 80, index % 5),
                _ => Word::spanning(index, index * 80, index % 5),
            })
            .collect();
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        let inspected = persistent::inspect_cache_entry(&persistent)
            .unwrap()
            .expect("published cache diagnostics");
        assert_eq!(inspected.word_count, words.len() as u64);
        assert!(inspected.block_count > 0);
        assert_eq!(
            inspected.total_bytes,
            inspected.data_bytes + inspected.index_bytes + 96
        );

        drop((writer, live_store));

        let reopened = IndexedAnnotationStore::open_persistent(&persistent)
            .unwrap()
            .expect("published cache");
        assert_eq!(reopened.metadata().total_word_count, words.len() as u64);
        assert_eq!(
            reopened
                .exact_window(0, u64::MAX, words.len() + 1)
                .unwrap()
                .annotations,
            direct_annotations(&words, 0, u64::MAX)
        );
        assert!(!reopened.presence_window(0, 4_000, 20).unwrap().is_empty());
    }

    #[test]
    fn committed_fingerprint_tracks_words_not_cache_identity() {
        let words: Vec<_> = (0..41)
            .map(|index| Word::spanning(index, index * 80, index % 5))
            .collect();
        let build = |cache_key, words: &[Word], batch_words: usize| {
            let config = persistent_config(cache_key);
            let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
            for batch in words.chunks(batch_words) {
                writer.append_batch(batch).unwrap();
            }
            writer.finish().unwrap();
            store.committed_data_fingerprint().unwrap()
        };

        let first = build([0x31; 32], &words, words.len());
        let second = build([0x32; 32], &words, 3);
        let mut changed = words.clone();
        changed[20].value ^= 1;
        let third = build([0x33; 32], &changed, 7);

        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn unfinished_persistent_store_never_becomes_discoverable() {
        let config = persistent_config([0x33; 32]);
        let persistent = config.persistence.clone().unwrap();
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.append(Word::new(1, 10)).unwrap();
        drop((writer, store));

        assert!(
            IndexedAnnotationStore::open_persistent(&persistent)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn persistent_cleanup_removes_unpinned_lru_entries() {
        let repository: Arc<dyn ArtifactRepository> =
            Arc::new(signal_artifacts::MemoryArtifactRepository::new());
        let first_key = [0x11; 32];
        let second_key = [0x22; 32];
        for key in [first_key, second_key] {
            let mut config = persistent_config(key);
            config = config.with_artifact_repository(Arc::clone(&repository));
            let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
            writer.append(Word::new(u64::from(key[0]), 10)).unwrap();
            writer.finish().unwrap();
            drop((writer, store));
        }
        let stats =
            persistent::cleanup_cache(&repository, 0, std::slice::from_ref(&second_key)).unwrap();

        assert_eq!(stats.entries, 1);
        assert_eq!(stats.removed_entries, 1);
        assert!(
            IndexedAnnotationStore::open_persistent(
                &PersistentStoreConfig::new(first_key)
                    .with_artifact_repository(Arc::clone(&repository))
            )
            .unwrap()
            .is_none()
        );
        assert!(
            IndexedAnnotationStore::open_persistent(
                &PersistentStoreConfig::new(second_key)
                    .with_artifact_repository(Arc::clone(&repository))
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn clearing_one_persistent_entry_keeps_other_cache_keys() {
        let repository: Arc<dyn ArtifactRepository> =
            Arc::new(signal_artifacts::MemoryArtifactRepository::new());
        let first = PersistentStoreConfig::new([0x31; 32])
            .with_artifact_repository(Arc::clone(&repository));
        let second = PersistentStoreConfig::new([0x32; 32])
            .with_artifact_repository(Arc::clone(&repository));
        for persistent in [&first, &second] {
            let mut config = test_config();
            config.persistence = Some(persistent.clone());
            let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
            writer.append(Word::new(1, 10)).unwrap();
            writer.finish().unwrap();
            drop((writer, store));
        }

        let stats = persistent::clear_cache_entry(&first).unwrap();
        assert_eq!(stats.removed_entries, 1);
        assert!(stats.removed_bytes > 0);
        assert!(
            IndexedAnnotationStore::open_persistent(&first)
                .unwrap()
                .is_none()
        );
        assert!(
            IndexedAnnotationStore::open_persistent(&second)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            persistent::clear_cache_entry(&first)
                .unwrap()
                .removed_entries,
            0
        );
    }

    #[test]
    fn finish_commits_partial_block_and_reads_it_by_directory_offset() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let words: Vec<_> = (0..7)
            .map(|index| Word::spanning(index, index * 10, index % 3))
            .collect();

        writer.append_batch(&words).unwrap();
        assert_eq!(store.snapshot().hot_tail.as_ref(), words.as_slice());
        writer.finish().unwrap();

        let snapshot = store.snapshot();
        assert_eq!(snapshot.metadata.status, StoreStatus::Finished);
        assert_eq!(snapshot.metadata.committed_block_count, 1);
        assert_eq!(snapshot.metadata.committed_word_count, words.len() as u64);
        assert!(snapshot.metadata.immutable_region_backed);
        assert!(snapshot.hot_tail.is_empty());
        assert_eq!(store.read_committed_block(0).unwrap().words, words);
    }

    #[test]
    fn configured_boundaries_create_multiple_ordered_blocks() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let words: Vec<_> = (0..41).map(|index| Word::new(index, index * 80)).collect();
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        let entries = store.directory();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].word_count, 16);
        assert_eq!(entries[1].word_count, 16);
        assert_eq!(entries[2].word_count, 9);
        let decoded: Vec<_> = (0..entries.len())
            .flat_map(|index| store.read_committed_block(index).unwrap().words)
            .collect();
        assert_eq!(decoded, words);
    }

    #[test]
    fn active_segment_keeps_committed_blocks_queryable_before_publication() {
        let mut config = test_config();
        config.block.max_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.target_segment_bytes = 1024;
        let expected = (0..4)
            .map(|index| Word::new(index, index * 10))
            .collect::<Vec<_>>();

        writer.append_batch(&expected).unwrap();
        writer.publish_hot_tail().unwrap();
        let segment = segment_key(store.shared.store_identity, 0).unwrap();
        assert!(store.shared.repository.open(&segment).unwrap().is_none());
        assert_eq!(
            store.exact_window(0, 40, 8).unwrap().annotations,
            direct_annotations(&expected, 0, 40)
        );

        writer.finish().unwrap();
        assert!(store.shared.repository.open(&segment).unwrap().is_some());
    }

    #[test]
    fn persistent_blocks_span_bounded_segments_and_reopen() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        let persistent = PersistentStoreConfig::new([0x91; 32])
            .with_artifact_repository(Arc::clone(&repository));
        let mut config = test_config().with_artifact_repository(Arc::clone(&repository));
        config.persistence = Some(persistent.clone());
        config.block.max_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.target_segment_bytes = 200;
        let expected = (0..12)
            .map(|index| Word::new(index, index * 10))
            .collect::<Vec<_>>();
        writer.append_batch(&expected).unwrap();
        writer.finish().unwrap();
        let directory = store.directory();
        let final_segment = directory
            .last()
            .expect("the test writes blocks")
            .segment_sequence;
        assert!(final_segment > 0);
        for sequence in 0..=final_segment {
            assert!(
                repository
                    .open(&segment_key(persistent.cache_key, sequence).unwrap())
                    .unwrap()
                    .is_some()
            );
        }
        drop(store);

        let reopened = IndexedAnnotationStore::open_persistent(&persistent)
            .unwrap()
            .expect("segmented persistent store should reopen");
        assert_eq!(
            reopened.exact_window(0, 120, 20).unwrap().annotations,
            direct_annotations(&expected, 0, 120)
        );
    }

    #[test]
    fn completed_blocks_publish_in_sequence_when_encoders_finish_out_of_order() {
        fn prepare(config: BlockCodecConfig, sequence: u64, words: &[Word]) -> PreparedBlock {
            let mut builder = WordBlockBuilder::new(config).unwrap();
            assert_eq!(builder.extend_ordered(words), words.len());
            let duration_free = builder.is_duration_free();
            let mut encoded = Vec::new();
            let result = builder.encode(sequence, &mut encoded).map(|metadata| {
                let summaries = word_presence_summaries(sequence, builder.words(), duration_free);
                (metadata, summaries)
            });
            builder.clear();
            PreparedBlock {
                builder,
                encoded,
                result,
            }
        }

        let config = test_config();
        let block_config = config.block;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let words: Vec<_> = (0..48).map(|index| Word::new(index, index * 80)).collect();
        let mut blocks = words
            .chunks(16)
            .enumerate()
            .map(|(sequence, words)| prepare(block_config, sequence as u64, words))
            .collect::<Vec<_>>();

        writer.in_flight_blocks = blocks.len();
        writer.next_dispatch_sequence = blocks.len() as u64;
        writer
            .accept_completed_block(BlockCompletion {
                sequence: 2,
                block: Some(blocks.pop().unwrap()),
            })
            .unwrap();
        writer.commit_ordered_blocks().unwrap();
        assert!(store.directory().is_empty());

        writer
            .accept_completed_block(BlockCompletion {
                sequence: 0,
                block: Some(blocks.remove(0)),
            })
            .unwrap();
        writer.commit_ordered_blocks().unwrap();
        assert_eq!(store.directory().len(), 1);

        writer
            .accept_completed_block(BlockCompletion {
                sequence: 1,
                block: Some(blocks.pop().unwrap()),
            })
            .unwrap();
        writer.commit_ordered_blocks().unwrap();
        writer.finish().unwrap();

        let entries = store.directory();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        let decoded: Vec<_> = (0..entries.len())
            .flat_map(|index| store.read_committed_block(index).unwrap().words)
            .collect();
        assert_eq!(decoded, words);
    }

    #[test]
    fn batched_append_publishes_all_completed_blocks_before_returning() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let words: Vec<_> = (0..91).map(|index| Word::new(index, index * 80)).collect();
        let batches = words.chunks(7).map(<[Word]>::to_vec).collect::<Vec<_>>();

        crate::derived_word_store::backend::AnnotationStoreWriterBackend::append_batches(
            &mut writer,
            &batches,
        )
        .unwrap();

        let snapshot = store.snapshot();
        assert_eq!(snapshot.metadata.committed_block_count, 5);
        assert_eq!(snapshot.metadata.committed_word_count, 80);
        assert_eq!(snapshot.hot_tail.as_ref(), &words[80..]);
        let exact = store.exact_window(0, u64::MAX, words.len()).unwrap();
        assert!(exact.complete);
        assert_eq!(exact.annotations, direct_annotations(&words, 0, u64::MAX));
    }

    #[test]
    fn exact_word_limit_commits_without_publishing_a_duplicate_hot_tail() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let words: Vec<_> = (0..16).map(|index| Word::new(index, index * 80)).collect();

        writer.append_batch(&words).unwrap();

        let snapshot = store.snapshot();
        assert_eq!(snapshot.metadata.committed_block_count, 1);
        assert_eq!(snapshot.metadata.committed_word_count, 16);
        assert_eq!(snapshot.metadata.hot_tail_word_count, 0);
    }

    #[test]
    fn concurrent_readers_never_observe_partial_commits() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let store = store.clone();
                let done = Arc::clone(&done);
                thread::spawn(move || {
                    let mut next_block = 0;
                    let mut words = Vec::new();
                    while !done.load(Ordering::Acquire)
                        || next_block < store.snapshot().metadata.committed_block_count
                    {
                        let block_count = store.snapshot().metadata.committed_block_count;
                        while next_block < block_count {
                            words.extend(store.read_committed_block(next_block).unwrap().words);
                            next_block += 1;
                        }
                        thread::yield_now();
                    }
                    words
                })
            })
            .collect();

        let expected: Vec<_> = (0..1_000)
            .map(|index| Word::new(index & 0xff, index * 80))
            .collect();
        for chunk in expected.chunks(37) {
            writer.append_batch(chunk).unwrap();
        }
        writer.finish().unwrap();
        done.store(true, Ordering::Release);

        for reader in readers {
            assert_eq!(reader.join().unwrap(), expected);
        }
    }

    #[test]
    fn out_of_order_input_fails_only_the_store() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        writer.append(Word::new(1, 10)).unwrap();
        assert!(matches!(
            writer.append(Word::new(2, 9)),
            Err(StoreError::Codec(CodecError::OutOfOrder { .. }))
        ));
        assert!(matches!(
            store.snapshot().metadata.status,
            StoreStatus::Failed(_)
        ));
        assert!(matches!(
            writer.append(Word::new(3, 11)),
            Err(StoreError::NotLive(StoreStatus::Failed(_)))
        ));
    }

    #[test]
    fn committed_read_error_is_reported_through_store_status() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        writer.append(Word::new(1, 0)).unwrap();
        writer.finish().unwrap();

        let entry = store.directory()[0];
        let key = segment_key(store.shared.store_identity, entry.segment_sequence).unwrap();
        let mut bytes = store.read_entry_bytes(entry).unwrap();
        bytes[super::super::format::BLOCK_HEADER_SIZE] ^= 0x80;
        let mut replacement = store.shared.repository.begin_write(key).unwrap();
        replacement.write_at(0, &bytes).unwrap();
        replacement.publish().unwrap();

        assert!(matches!(
            store.read_committed_block(0),
            Err(StoreError::Codec(CodecError::ChecksumMismatch { .. }))
        ));
        assert!(matches!(
            store.snapshot().metadata.status,
            StoreStatus::Failed(_)
        ));
    }

    #[test]
    fn cancellation_discards_the_unpublished_active_segment_promptly() {
        let mut config = test_config();
        config.block.max_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer.append(Word::new(1, 0)).unwrap();
        let key = segment_key(store.shared.store_identity, 0).unwrap();

        let start = Instant::now();
        writer.cancel().unwrap();
        drop(writer);
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(store.snapshot().metadata.status, StoreStatus::Cancelled);
        assert!(store.shared.repository.open(&key).unwrap().is_none());
        assert!(store.shared.pending_blocks.read().unwrap().is_empty());

        let repository = Arc::clone(&store.shared.repository);
        drop(store);
        assert!(repository.open(&key).unwrap().is_none());
    }

    #[test]
    fn exact_query_combines_committed_blocks_and_the_live_hot_tail() {
        let mut config = test_config();
        config.block.max_words = 4;
        config.hot_tail_publish_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let words: Vec<_> = (0..10)
            .map(|index| {
                if index == 6 {
                    Word::spanning(index, index * 10, 7)
                } else {
                    Word::new(index, index * 10)
                }
            })
            .collect();
        writer.append_batch(&words).unwrap();

        let result = store.exact_window(15, 75, 100).unwrap();
        assert!(result.complete);
        assert_eq!(result.annotations, direct_annotations(&words, 15, 75));
        assert_eq!(store.snapshot().metadata.committed_block_count, 2);
        assert_eq!(store.snapshot().metadata.hot_tail_word_count, 2);
    }

    #[test]
    fn exact_query_reports_an_incomplete_limited_window() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        let words: Vec<_> = (0..100).map(|index| Word::new(index, index * 10)).collect();
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        let result = store.exact_window(0, 1_000, 7).unwrap();
        assert!(!result.complete);
        assert_eq!(result.annotations.len(), 7);
    }

    #[test]
    fn instantaneous_words_do_not_bridge_a_long_decoding_gap() {
        let mut config = test_config();
        config.block.max_words = 1;
        config.hot_tail_publish_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer
            .append_batch(&[Word::new(1, 0), Word::new(2, 100), Word::new(3, 10_000)])
            .unwrap();

        let burst = store.exact_window(0, 500, 10).unwrap();
        assert_eq!(burst.annotations[0].end_ns, 100);
        assert_eq!(burst.annotations[1].end_ns, 200);
        let inside_gap = store.exact_window(1_000, 9_000, 10).unwrap();
        assert!(inside_gap.annotations.is_empty());
        let panned_left = store.exact_window(0, 9_000, 10).unwrap();
        assert!(
            panned_left
                .annotations
                .iter()
                .all(|annotation| !(annotation.start_ns <= 5_000 && annotation.end_ns >= 5_000)),
            "panning left must not change whether the middle of the gap is empty"
        );
        assert_eq!(store.nearest_boundary(205, 10).unwrap(), Some(200));
    }

    #[test]
    fn nearest_boundary_checks_starts_explicit_ends_and_ties() {
        let mut config = test_config();
        config.block.max_words = 2;
        config.hot_tail_publish_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer
            .append_batch(&[Word::new(1, 10), Word::spanning(2, 30, 5), Word::new(3, 50)])
            .unwrap();

        assert_eq!(store.nearest_boundary(33, 10).unwrap(), Some(35));
        assert_eq!(store.nearest_boundary(20, 10).unwrap(), Some(10));
        assert_eq!(store.nearest_boundary(100, 20).unwrap(), None);
    }

    #[test]
    fn exact_and_boundary_queries_find_a_partial_word_spanning_later_blocks() {
        let mut config = test_config();
        config.block.max_words = 2;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let mut words = vec![Word::spanning(0x27, 10, 9_990)];
        words.extend((1..=9).map(|index| Word::new(index, index * 100)));
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        let window = store.exact_window(9_000, 9_500, 10).unwrap();
        assert!(window.complete);
        assert_eq!(
            window.annotations,
            vec![Annotation {
                start_ns: 10,
                end_ns: 10_000,
                value: 0x27,
                payload: None,
            }]
        );
        assert_eq!(store.nearest_boundary(9_998, 10).unwrap(), Some(10_000));
    }

    #[test]
    fn exact_and_boundary_queries_match_randomized_reference() {
        let mut config = test_config();
        config.block.max_words = 31;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let mut random = 0x243f_6a88_85a3_08d3u64;
        let mut timestamp = 0u64;
        let mut words = Vec::new();
        for index in 0..2_000 {
            timestamp += next_random(&mut random) % 20;
            let duration = if index % 23 == 0 {
                next_random(&mut random) % 10 + 1
            } else {
                0
            };
            words.push(Word::spanning(
                next_random(&mut random),
                timestamp,
                duration,
            ));
        }
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        for _ in 0..250 {
            let start = next_random(&mut random) % (timestamp + 100);
            let end = start + next_random(&mut random) % 500;
            let expected = direct_annotations(&words, start, end);
            let actual = store.exact_window(start, end, 10_000).unwrap();
            assert!(actual.complete);
            assert_eq!(actual.annotations, expected, "window {start}..={end}");

            let target = next_random(&mut random) % (timestamp + 100);
            let max_distance = next_random(&mut random) % 100;
            assert_eq!(
                store.nearest_boundary(target, max_distance).unwrap(),
                direct_nearest_boundary(&words, target, max_distance),
                "target={target}, max_distance={max_distance}"
            );
        }
    }

    #[test]
    fn exact_query_populates_the_process_decoded_block_cache() {
        let (mut writer, store) = IndexedAnnotationWriter::create(test_config()).unwrap();
        writer
            .append_batch(&[Word::new(1, 10), Word::new(2, 20)])
            .unwrap();
        writer.finish().unwrap();
        assert!(!cache_contains(store.shared.store_id, 0));

        store.exact_window(0, 30, 10).unwrap();
        assert!(cache_contains(store.shared.store_id, 0));
    }

    #[test]
    fn dense_presence_uses_summaries_and_hot_tail_without_decoding_blocks() {
        let mut config = test_config();
        config.block.max_words = 16;
        config.hot_tail_publish_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let words: Vec<_> = (0..40).map(|index| Word::new(index, index * 100)).collect();
        writer.append_batch(&words).unwrap();
        assert_eq!(store.snapshot().metadata.committed_block_count, 2);
        assert_eq!(store.snapshot().metadata.hot_tail_word_count, 8);
        assert!(!cache_contains(store.shared.store_id, 0));
        assert!(!cache_contains(store.shared.store_id, 1));

        // One target bucket permits at most 32 refined words, so this
        // forty-word window must take the summary fallback.
        let buckets = store.presence_window(0, 3_999, 1).unwrap();
        assert!(buckets.len() <= 1);
        assert!(buckets.iter().any(|bucket| bucket.end_ns >= 3_900));
        assert!(!cache_contains(store.shared.store_id, 0));
        assert!(!cache_contains(store.shared.store_id, 1));
    }

    #[test]
    fn presence_query_does_not_fill_a_large_inter_block_gap() {
        let mut config = test_config();
        config.block.max_words = 64;
        config.block.max_inter_word_gap_ns = 100;
        config.hot_tail_publish_words = 1;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        writer
            .append_batch(&[Word::new(1, 0), Word::new(2, 10), Word::new(3, 10_000)])
            .unwrap();

        let buckets = store.presence_window(0, 10_009, 10).unwrap();
        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[0].start_ns, 0);
        assert_eq!(buckets[1].end_ns, 10_009);
    }

    #[test]
    fn refined_presence_keeps_visible_gaps_inside_one_encoded_block() {
        let mut config = test_config();
        config.block.max_words = 1_000;
        config.block.max_inter_word_gap_ns = u64::MAX;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let mut words: Vec<_> = (0..150).map(|timestamp| Word::new(1, timestamp)).collect();
        words.extend((10_000..10_150).map(|timestamp| Word::new(2, timestamp)));
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        assert_eq!(store.snapshot().metadata.committed_block_count, 1);
        assert!(!store.exact_window(0, 10_149, 200).unwrap().complete);
        let buckets = store.presence_window(0, 10_149, 100).unwrap();
        assert!(
            buckets
                .iter()
                .all(|bucket| !(bucket.start_ns <= 5_000 && bucket.end_ns >= 5_000)),
            "presence refinement must not smear one block across its internal gap"
        );
    }

    #[test]
    fn coarse_presence_keeps_visible_gaps_inside_one_encoded_block_without_decoding() {
        let mut config = test_config();
        config.block.max_words = 20_000;
        config.block.max_inter_word_gap_ns = u64::MAX;
        let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
        let mut words: Vec<_> = (0..5_000).map(|index| Word::new(1, index * 10)).collect();
        words.extend((0..5_000).map(|index| Word::new(2, 100_000 + index * 10)));
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();

        assert_eq!(store.snapshot().metadata.committed_block_count, 1);
        assert_eq!(
            store.shared.state.read().unwrap().presence.leaves().len(),
            2
        );
        assert!(!cache_contains(store.shared.store_id, 0));
        let buckets = store.presence_window(0, 149_990, 100).unwrap();
        assert!(
            buckets
                .iter()
                .all(|bucket| !(bucket.start_ns <= 75_000 && bucket.end_ns >= 75_000)),
            "coarse summaries must preserve an inactive interval inside one block"
        );
        assert!(!cache_contains(store.shared.store_id, 0));
    }

    #[test]
    fn presence_run_limit_preserves_the_largest_inactive_gaps() {
        let mut timestamp = 0u64;
        let mut words = Vec::new();
        for index in 0..1_000 {
            words.push(Word::new(index, timestamp));
            timestamp += if index == 499 { 100_000_000 } else { 2_000_000 };
        }

        let summaries = word_presence_summaries(0, &words, true);
        assert_eq!(summaries.len(), MAX_PRESENCE_RUNS_PER_BLOCK);
        assert_eq!(
            summaries
                .iter()
                .map(|summary| summary.word_count)
                .sum::<u64>(),
            words.len() as u64
        );
        let gap_midpoint = words[499].timestamp_ns + 50_000_000;
        assert!(
            summaries.iter().all(|summary| {
                !(summary.start_ns <= gap_midpoint && summary.end_ns >= gap_midpoint)
            }),
            "the largest inactive interval must survive bounded coalescing"
        );
    }

    #[test]
    fn duration_free_presence_fast_path_matches_general_summaries() {
        let mut timestamp = 0u64;
        let words: Vec<_> = (0..2_000)
            .map(|index| {
                timestamp = timestamp.saturating_add(match index % 11 {
                    0 => 0,
                    1 => 5_000_000,
                    2 => 25,
                    3 => 50,
                    _ => 20,
                });
                Word::new(index, timestamp)
            })
            .collect();

        assert_eq!(
            word_presence_summaries(7, &words, true),
            word_presence_summaries(7, &words, false)
        );
    }

    #[test]
    fn persistent_presence_reopens_multiple_runs_for_one_encoded_block() {
        let cache_key = [0x7b; 32];
        let mut config = persistent_config(cache_key);
        config.block.max_words = 20_000;
        config.block.max_inter_word_gap_ns = u64::MAX;
        let persistent = config.persistence.clone().unwrap();
        let (mut writer, live_store) = IndexedAnnotationWriter::create(config).unwrap();
        let mut words: Vec<_> = (0..5_000).map(|index| Word::new(1, index * 10)).collect();
        words.extend((0..5_000).map(|index| Word::new(2, 100_000 + index * 10)));
        writer.append_batch(&words).unwrap();
        writer.finish().unwrap();
        drop((writer, live_store));

        let reopened = IndexedAnnotationStore::open_persistent(&persistent)
            .unwrap()
            .expect("published cache");
        assert_eq!(reopened.snapshot().metadata.committed_block_count, 1);
        assert_eq!(
            reopened
                .shared
                .state
                .read()
                .unwrap()
                .presence
                .leaves()
                .len(),
            2
        );
        let buckets = reopened.presence_window(0, 149_990, 100).unwrap();
        assert!(
            buckets
                .iter()
                .all(|bucket| !(bucket.start_ns <= 75_000 && bucket.end_ns >= 75_000)),
            "reopened presence summaries must retain internal gaps"
        );
        assert!(!cache_contains(reopened.shared.store_id, 0));
    }

    fn direct_annotations(words: &[Word], start_ns: u64, end_ns: u64) -> Vec<Annotation> {
        words
            .iter()
            .enumerate()
            .filter_map(|(index, word)| {
                let annotation_end = if word.duration_ns != 0 {
                    word.timestamp_ns.saturating_add(word.duration_ns)
                } else {
                    words.get(index + 1).map_or(word.timestamp_ns, |next| {
                        instantaneous_word_end_ns(
                            index
                                .checked_sub(1)
                                .map(|previous| words[previous].timestamp_ns),
                            word.timestamp_ns,
                            next.timestamp_ns,
                        )
                    })
                };
                (word.timestamp_ns <= end_ns && annotation_end >= start_ns).then_some(Annotation {
                    start_ns: word.timestamp_ns,
                    end_ns: annotation_end,
                    value: word.value,
                    payload: word.payload.clone(),
                })
            })
            .collect()
    }

    fn direct_nearest_boundary(words: &[Word], target: u64, max_distance: u64) -> Option<u64> {
        words
            .iter()
            .enumerate()
            .flat_map(|(index, word)| {
                [
                    Some(word.timestamp_ns),
                    if word.duration_ns != 0 {
                        Some(word.timestamp_ns.saturating_add(word.duration_ns))
                    } else {
                        words.get(index + 1).map(|next| {
                            instantaneous_word_end_ns(
                                index
                                    .checked_sub(1)
                                    .map(|previous| words[previous].timestamp_ns),
                                word.timestamp_ns,
                                next.timestamp_ns,
                            )
                        })
                    },
                ]
            })
            .flatten()
            .filter_map(|boundary| {
                let distance = boundary.abs_diff(target);
                (distance <= max_distance).then_some((boundary, distance))
            })
            .min_by_key(|&(boundary, distance)| (distance, boundary))
            .map(|(boundary, _)| boundary)
    }

    fn next_random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }
}
