//! Encoded in-memory derived-word store for wasm.
//!
//! This target-specific backing stores the same encoded blocks and presence
//! summaries as native. Browser persistence is selected above this store; it
//! does not change the word format or query semantics.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use super::super::super::codec::{WordBlockBuilder, decode_word_block};
use super::super::super::config::{LiveStoreConfig, PersistentStoreConfig};
use super::super::super::errors::CodecError;
use super::super::super::format::BlockDirectoryEntry;
use super::super::super::presence::{WordPresenceIndex, word_presence_summaries};
use super::super::super::query::{
    AnnotationQuery, AnnotationQueryError, AnnotationQueryResult, AnnotationStoreMetadata,
    ExactAnnotationWindow, WordPresenceBucket, annotation_window_from_ordered_words,
    boundary_block_indices, exact_block_indices, nearest_boundary_from_ordered_words,
};
use super::super::super::state::{LiveStoreMetadata, LiveStoreSnapshot, StoreStatus};
use crate::events::Word;

pub(crate) fn default_working_directory() -> PathBuf {
    PathBuf::new()
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("derived-word store codec error: {0}")]
    Codec(#[from] CodecError),

    #[error("derived-word store is not live: {0:?}")]
    NotLive(StoreStatus),

    #[error("persistent derived-word storage is unavailable on wasm")]
    PersistenceUnsupported,
}

pub type StoreResult<T> = std::result::Result<T, StoreError>;

struct MemoryState {
    directory: Vec<BlockDirectoryEntry>,
    encoded_blocks: Vec<Arc<[u8]>>,
    presence: WordPresenceIndex,
    generation: u64,
    committed_word_count: u64,
    committed_data_len: u64,
    committed_first_timestamp_ns: Option<u64>,
    committed_last_timestamp_ns: Option<u64>,
    hot_tail: Arc<[Word]>,
    status: StoreStatus,
}

#[derive(Clone)]
pub struct IndexedAnnotationStore {
    state: Arc<RwLock<MemoryState>>,
}

impl IndexedAnnotationStore {
    pub fn open_persistent(
        _config: &PersistentStoreConfig,
    ) -> StoreResult<Option<IndexedAnnotationStore>> {
        Ok(None)
    }

    pub fn snapshot(&self) -> LiveStoreSnapshot {
        let state = self.state.read().unwrap();
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
                mmap_backed: false,
                persistent_cache: false,
                status: state.status.clone(),
            },
            hot_tail: Arc::clone(&state.hot_tail),
        }
    }

    fn exact_word_context(
        &self,
        start_ns: u64,
        end_ns: u64,
    ) -> (u64, Vec<Arc<[u8]>>, Arc<[Word]>, usize) {
        let state = self.state.read().unwrap();
        let indices = exact_block_indices(&state.directory, &state.presence, start_ns, end_ns);
        let (blocks, committed_word_count) = selected_blocks(&state, &indices);
        (
            state.generation,
            blocks,
            Arc::clone(&state.hot_tail),
            committed_word_count,
        )
    }

    fn boundary_word_context(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> (Vec<Arc<[u8]>>, Arc<[Word]>, usize) {
        let state = self.state.read().unwrap();
        let indices = boundary_block_indices(
            &state.directory,
            &state.presence,
            timestamp_ns,
            max_distance_ns,
        );
        let (blocks, committed_word_count) = selected_blocks(&state, &indices);
        (blocks, Arc::clone(&state.hot_tail), committed_word_count)
    }
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
        if start_ns > end_ns {
            return Err(AnnotationQueryError::InvalidWindow { start_ns, end_ns });
        }
        if target_buckets == 0 {
            return Err(AnnotationQueryError::ZeroBucketLimit);
        }
        let mut buckets = {
            let state = self.state.read().unwrap();
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
        let (generation, blocks, hot_tail, committed_word_count) =
            self.exact_word_context(start_ns, end_ns);
        let words = decode_context(&blocks, hot_tail, committed_word_count)?;
        let (annotations, truncated) =
            annotation_window_from_ordered_words(&words, start_ns, end_ns, max_words);
        Ok(ExactAnnotationWindow {
            annotations,
            complete: !truncated,
            generation,
        })
    }

    fn nearest_boundary(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> AnnotationQueryResult<Option<u64>> {
        let (blocks, hot_tail, committed_word_count) =
            self.boundary_word_context(timestamp_ns, max_distance_ns);
        let words = decode_context(&blocks, hot_tail, committed_word_count)?;
        Ok(nearest_boundary_from_ordered_words(
            &words,
            timestamp_ns,
            max_distance_ns,
        ))
    }
}

pub struct IndexedAnnotationWriter {
    store: IndexedAnnotationStore,
    builder: WordBlockBuilder,
    next_sequence: u64,
    next_data_offset: u64,
    last_timestamp_ns: Option<u64>,
    appended_word_count: u64,
}

impl IndexedAnnotationWriter {
    pub fn create(config: LiveStoreConfig) -> StoreResult<(Self, IndexedAnnotationStore)> {
        if config.hot_tail_publish_words == 0 {
            return Err(StoreError::Codec(CodecError::InvalidConfiguration(
                "hot_tail_publish_words must be greater than zero",
            )));
        }
        let store = IndexedAnnotationStore {
            state: Arc::new(RwLock::new(MemoryState {
                directory: Vec::new(),
                encoded_blocks: Vec::new(),
                presence: WordPresenceIndex::new(),
                generation: 0,
                committed_word_count: 0,
                committed_data_len: 0,
                committed_first_timestamp_ns: None,
                committed_last_timestamp_ns: None,
                hot_tail: Arc::from([]),
                status: StoreStatus::Live,
            })),
        };
        let writer = Self {
            store: store.clone(),
            builder: WordBlockBuilder::new(config.block)?,
            next_sequence: 0,
            next_data_offset: 0,
            last_timestamp_ns: None,
            appended_word_count: 0,
        };
        Ok((writer, store))
    }

    pub fn store(&self) -> IndexedAnnotationStore {
        self.store.clone()
    }

    pub fn append(&mut self, word: Word) -> StoreResult<()> {
        self.append_batch(std::slice::from_ref(&word))
    }

    pub fn append_batch(&mut self, words: &[Word]) -> StoreResult<()> {
        self.ensure_live()?;
        self.validate_order(words)?;
        let mut remaining = words;
        while !remaining.is_empty() {
            let accepted = self.builder.extend_ordered(remaining);
            self.appended_word_count = self.appended_word_count.saturating_add(accepted as u64);
            remaining = &remaining[accepted..];
            if !remaining.is_empty() || self.builder.is_at_word_limit() {
                self.commit_current_block()?;
            }
        }
        self.publish_hot_tail_inner();
        Ok(())
    }

    pub fn publish_hot_tail(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        self.publish_hot_tail_inner();
        Ok(())
    }

    pub fn finish(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        self.commit_current_block()?;
        let mut state = self.store.state.write().unwrap();
        state.status = StoreStatus::Finished;
        state.generation += 1;
        Ok(())
    }

    pub fn cancel(&mut self) -> StoreResult<()> {
        self.ensure_live()?;
        self.builder.clear();
        let mut state = self.store.state.write().unwrap();
        state.directory.clear();
        state.encoded_blocks.clear();
        state.presence = WordPresenceIndex::new();
        state.committed_word_count = 0;
        state.committed_data_len = 0;
        state.committed_first_timestamp_ns = None;
        state.committed_last_timestamp_ns = None;
        state.hot_tail = Arc::from([]);
        state.status = StoreStatus::Cancelled;
        state.generation += 1;
        Ok(())
    }

    fn validate_order(&mut self, words: &[Word]) -> StoreResult<()> {
        let mut previous_timestamp_ns = self.last_timestamp_ns;
        for (offset, word) in words.iter().enumerate() {
            if let Some(previous_timestamp_ns) = previous_timestamp_ns
                && word.timestamp_ns < previous_timestamp_ns
            {
                return Err(StoreError::Codec(CodecError::OutOfOrder {
                    index: usize::try_from(self.appended_word_count)
                        .unwrap_or(usize::MAX)
                        .saturating_add(offset),
                    previous_timestamp_ns,
                    timestamp_ns: word.timestamp_ns,
                }));
            }
            previous_timestamp_ns = Some(word.timestamp_ns);
        }
        self.last_timestamp_ns = previous_timestamp_ns;
        Ok(())
    }

    fn commit_current_block(&mut self) -> StoreResult<()> {
        if self.builder.is_empty() {
            return Ok(());
        }
        let duration_free = self.builder.is_duration_free();
        let summaries =
            word_presence_summaries(self.next_sequence, self.builder.words(), duration_free);
        let mut encoded = Vec::new();
        let metadata = self.builder.encode(self.next_sequence, &mut encoded)?;
        let header = metadata.header;
        let entry = BlockDirectoryEntry {
            sequence: header.sequence,
            first_timestamp_ns: header.first_timestamp_ns,
            last_timestamp_ns: header.last_timestamp_ns,
            data_offset: self.next_data_offset,
            block_len: header.block_len,
            word_count: header.word_count,
            value_bytes: header.value_bytes,
            flags: header.flags as u8,
        };
        {
            let mut state = self.store.state.write().unwrap();
            state.directory.push(entry);
            state.encoded_blocks.push(Arc::from(encoded));
            for summary in summaries {
                state.presence.push(summary);
            }
            state.committed_word_count += u64::from(entry.word_count);
            state.committed_data_len += u64::from(entry.block_len);
            state
                .committed_first_timestamp_ns
                .get_or_insert(entry.first_timestamp_ns);
            state.committed_last_timestamp_ns = Some(entry.last_timestamp_ns);
            state.hot_tail = Arc::from([]);
            state.generation += 1;
        }
        self.next_sequence += 1;
        self.next_data_offset += u64::from(entry.block_len);
        self.builder.clear();
        Ok(())
    }

    fn publish_hot_tail_inner(&mut self) {
        let hot_tail: Arc<[Word]> = Arc::from(self.builder.words().to_vec());
        let mut state = self.store.state.write().unwrap();
        state.hot_tail = hot_tail;
        state.generation += 1;
    }

    fn ensure_live(&self) -> StoreResult<()> {
        let state = self.store.state.read().unwrap();
        if state.status == StoreStatus::Live {
            Ok(())
        } else {
            Err(StoreError::NotLive(state.status.clone()))
        }
    }
}

fn decode_context(
    blocks: &[Arc<[u8]>],
    hot_tail: Arc<[Word]>,
    committed_word_count: usize,
) -> AnnotationQueryResult<Vec<Word>> {
    let mut words = Vec::with_capacity(committed_word_count.saturating_add(hot_tail.len()));
    for bytes in blocks {
        let block = decode_word_block(bytes)
            .map_err(|error| AnnotationQueryError::Store(error.to_string()))?;
        words.extend(block.words);
    }
    words.extend_from_slice(&hot_tail);
    Ok(words)
}

fn selected_blocks(state: &MemoryState, indices: &[usize]) -> (Vec<Arc<[u8]>>, usize) {
    let mut blocks = Vec::with_capacity(indices.len());
    let mut committed_word_count = 0usize;
    for &index in indices {
        committed_word_count =
            committed_word_count.saturating_add(state.directory[index].word_count as usize);
        blocks.push(Arc::clone(&state.encoded_blocks[index]));
    }
    (blocks, committed_word_count)
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

impl super::super::super::backend::AnnotationStoreBackend for IndexedAnnotationStore {
    fn snapshot(&self) -> LiveStoreSnapshot {
        IndexedAnnotationStore::snapshot(self)
    }
}

impl super::super::super::backend::AnnotationStoreWriterBackend for IndexedAnnotationWriter {
    fn append_batch(&mut self, words: &[Word]) -> StoreResult<()> {
        IndexedAnnotationWriter::append_batch(self, words)
    }

    fn finish(&mut self) -> StoreResult<()> {
        IndexedAnnotationWriter::finish(self)
    }
}

#[cfg(test)]
mod wasm_store_tests {
    use super::super::super::super::config::BlockCodecConfig;
    use super::*;

    fn config() -> LiveStoreConfig {
        LiveStoreConfig {
            block: BlockCodecConfig {
                max_words: 2,
                ..BlockCodecConfig::default()
            },
            hot_tail_publish_words: 1,
            ..LiveStoreConfig::default()
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn committed_words_are_retained_as_shared_encoded_blocks() {
        let (mut writer, store) = IndexedAnnotationWriter::create(config()).unwrap();
        writer
            .append_batch(&[
                Word::new(0x11, 100),
                Word::new(0x22, 200),
                Word::new(0x33, 300),
            ])
            .unwrap();

        let snapshot = store.snapshot();
        assert_eq!(snapshot.metadata.committed_block_count, 1);
        assert_eq!(snapshot.metadata.committed_word_count, 2);
        assert_eq!(snapshot.metadata.hot_tail_word_count, 1);
        assert!(snapshot.metadata.committed_data_len > 0);
        let state = store.state.read().unwrap();
        assert_eq!(state.directory[0].word_count, 2);
        assert!(!state.encoded_blocks[0].is_empty());
        drop(state);

        writer.finish().unwrap();
        let snapshot = store.snapshot();
        assert_eq!(snapshot.metadata.committed_block_count, 2);
        assert_eq!(snapshot.metadata.committed_word_count, 3);
        assert_eq!(snapshot.metadata.hot_tail_word_count, 0);
        assert_eq!(
            store
                .exact_window(0, 400, 10)
                .unwrap()
                .annotations
                .iter()
                .map(|annotation| annotation.value)
                .collect::<Vec<_>>(),
            vec![0x11, 0x22, 0x33]
        );
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn decoded_queries_span_committed_blocks_and_the_live_tail() {
        let (mut writer, store) = IndexedAnnotationWriter::create(config()).unwrap();
        writer
            .append_batch(&[
                Word::spanning(0x11, 100, 20),
                Word::new(0x22, 200),
                Word::new(0x33, 300),
            ])
            .unwrap();

        let exact = store.exact_window(0, 400, 10).unwrap();
        assert!(exact.complete);
        assert_eq!(exact.annotations.len(), 3);
        assert_eq!(exact.annotations[0].end_ns, 120);
        assert_eq!(store.nearest_boundary(298, 5).unwrap(), Some(300));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn narrow_queries_do_not_decode_unrelated_committed_blocks() {
        let (mut writer, store) = IndexedAnnotationWriter::create(config()).unwrap();
        writer
            .append_batch(&[
                Word::new(0x11, 100),
                Word::new(0x22, 300),
                Word::new(0x33, 500),
                Word::new(0x44, 700),
                Word::new(0x55, 900),
            ])
            .unwrap();
        writer.finish().unwrap();
        {
            let mut state = store.state.write().unwrap();
            state.encoded_blocks[2] = Arc::from([0u8]);
        }

        assert_eq!(
            store
                .exact_window(90, 110, 10)
                .unwrap()
                .annotations
                .iter()
                .map(|annotation| annotation.value)
                .collect::<Vec<_>>(),
            vec![0x11]
        );
    }
}
