use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::codec::DecodedWordBlock;
use super::format::RestartEntry;

const DEFAULT_DECODED_BLOCK_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Current usage and activity counters for one decoded-block cache instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodedBlockCacheStats {
    /// Number of decoded-block lookups served from this cache.
    pub hits: u64,
    /// Number of decoded-block lookups not present in this cache.
    pub misses: u64,
    /// Number of immutable decoded blocks currently retained.
    pub entries: usize,
    /// Estimated bytes currently retained by decoded blocks.
    pub memory_bytes: usize,
    /// Maximum estimated bytes this cache may retain.
    pub budget_bytes: usize,
}

/// Cloneable owner of one bounded decoded annotation-block cache.
///
/// Clones share one cache instance. Constructing another handle creates an
/// independent cache, allowing each application or isolated test to own its
/// complete decoded-block lifetime and statistics.
#[derive(Clone)]
pub struct DecodedBlockCacheHandle {
    cache: Arc<Mutex<DecodedBlockCache>>,
}

impl DecodedBlockCacheHandle {
    /// Creates an empty decoded-block cache with a fixed byte budget.
    ///
    /// A zero-byte budget explicitly disables decoded-block retention while
    /// preserving miss statistics.
    ///
    /// # Parameters
    /// - `budget_bytes`: Maximum estimated bytes retained by decoded blocks.
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            cache: Arc::new(Mutex::new(DecodedBlockCache::new(budget_bytes))),
        }
    }

    /// Returns a coherent usage and activity snapshot for this cache instance.
    pub fn stats(&self) -> DecodedBlockCacheStats {
        self.cache.lock().unwrap().stats()
    }

    /// Clears hit and miss counters without evicting decoded blocks.
    pub fn reset_stats(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.hits = 0;
        cache.misses = 0;
    }

    /// Evicts every decoded block owned by this cache instance.
    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.entries.clear();
        cache.memory_bytes = 0;
    }

    pub(crate) fn cached_block(
        &self,
        store_id: u64,
        sequence: u64,
    ) -> Option<Arc<DecodedWordBlock>> {
        self.cache
            .lock()
            .unwrap()
            .get(CacheKey { store_id, sequence })
    }

    pub(crate) fn cache_block(&self, store_id: u64, block: Arc<DecodedWordBlock>) {
        let sequence = block.header.sequence;
        self.cache
            .lock()
            .unwrap()
            .insert(CacheKey { store_id, sequence }, block);
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, store_id: u64, sequence: u64) -> bool {
        self.cache
            .lock()
            .unwrap()
            .entries
            .contains_key(&CacheKey { store_id, sequence })
    }
}

impl Default for DecodedBlockCacheHandle {
    fn default() -> Self {
        Self::new(DEFAULT_DECODED_BLOCK_CACHE_BYTES)
    }
}

impl fmt::Debug for DecodedBlockCacheHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DecodedBlockCacheHandle")
            .field("stats", &self.stats())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    store_id: u64,
    sequence: u64,
}

struct CacheEntry {
    block: Arc<DecodedWordBlock>,
    memory_bytes: usize,
    last_access: u64,
}

struct DecodedBlockCache {
    entries: HashMap<CacheKey, CacheEntry>,
    memory_bytes: usize,
    budget_bytes: usize,
    access_clock: u64,
    hits: u64,
    misses: u64,
}

impl DecodedBlockCache {
    fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            memory_bytes: 0,
            budget_bytes,
            access_clock: 0,
            hits: 0,
            misses: 0,
        }
    }

    fn get(&mut self, key: CacheKey) -> Option<Arc<DecodedWordBlock>> {
        self.access_clock = self.access_clock.wrapping_add(1);
        let Some(entry) = self.entries.get_mut(&key) else {
            self.misses += 1;
            return None;
        };
        self.hits += 1;
        entry.last_access = self.access_clock;
        Some(Arc::clone(&entry.block))
    }

    fn insert(&mut self, key: CacheKey, block: Arc<DecodedWordBlock>) {
        let memory_bytes = decoded_block_bytes(&block);
        if memory_bytes > self.budget_bytes {
            return;
        }
        self.access_clock = self.access_clock.wrapping_add(1);
        if let Some(previous) = self.entries.remove(&key) {
            self.memory_bytes -= previous.memory_bytes;
        }
        self.memory_bytes += memory_bytes;
        self.entries.insert(
            key,
            CacheEntry {
                block,
                memory_bytes,
                last_access: self.access_clock,
            },
        );
        self.evict_to_budget();
    }

    fn evict_to_budget(&mut self) {
        while self.memory_bytes > self.budget_bytes {
            let Some((&oldest_key, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
            else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest_key) {
                self.memory_bytes -= removed.memory_bytes;
            }
        }
    }

    fn stats(&self) -> DecodedBlockCacheStats {
        DecodedBlockCacheStats {
            hits: self.hits,
            misses: self.misses,
            entries: self.entries.len(),
            memory_bytes: self.memory_bytes,
            budget_bytes: self.budget_bytes,
        }
    }
}

fn decoded_block_bytes(block: &DecodedWordBlock) -> usize {
    size_of::<DecodedWordBlock>()
        + block.words.capacity() * size_of::<crate::events::Word>()
        + block.restarts.capacity() * size_of::<RestartEntry>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derived_word_store::format::WordBlockHeader;
    use crate::events::Word;

    fn block(sequence: u64, words: usize) -> Arc<DecodedWordBlock> {
        Arc::new(DecodedWordBlock {
            header: WordBlockHeader {
                flags: 0,
                sequence,
                first_timestamp_ns: 0,
                last_timestamp_ns: words.saturating_sub(1) as u64,
                word_count: words as u32,
                value_bytes: 1,
                record_payload_len: 0,
                restart_count: 0,
                restart_table_offset: 0,
                duration_count: 0,
                duration_table_offset: 0,
                block_len: 0,
                crc32c: 0,
            },
            restarts: Vec::new(),
            words: (0..words)
                .map(|timestamp| Word::new(0, timestamp as u64))
                .collect(),
        })
    }

    #[test]
    fn byte_budget_evicts_the_least_recently_used_block() {
        let first = block(1, 32);
        let second = block(2, 32);
        let third = block(3, 32);
        let one_block = decoded_block_bytes(&first);
        let mut cache = DecodedBlockCache::new(one_block * 2);
        let key = |sequence| CacheKey {
            store_id: 7,
            sequence,
        };

        cache.insert(key(1), first);
        cache.insert(key(2), second);
        assert!(cache.get(key(1)).is_some());
        cache.insert(key(3), third);

        assert!(cache.get(key(1)).is_some());
        assert!(cache.get(key(2)).is_none());
        assert!(cache.get(key(3)).is_some());
        assert!(cache.memory_bytes <= cache.budget_bytes);
    }

    #[test]
    fn separately_constructed_handles_isolate_entries_and_statistics() {
        let first = DecodedBlockCacheHandle::new(1024 * 1024);
        let second = DecodedBlockCacheHandle::new(1024 * 1024);

        first.cache_block(7, block(1, 8));
        assert!(first.cached_block(7, 1).is_some());
        assert!(second.cached_block(7, 1).is_none());
        assert_eq!(first.stats().entries, 1);
        assert_eq!(first.stats().hits, 1);
        assert_eq!(second.stats().entries, 0);
        assert_eq!(second.stats().misses, 1);
    }
}
