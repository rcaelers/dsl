use crate::derived_index::MipmapRecord;
use crate::derived_word_store::{
    AnnotationQuery, IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig, StoreStatus,
};
use crate::events::{Annotation, Word};
use crate::payload::{CollectedLaneStorageBacking, CollectedLaneStorageSnapshot};

pub(crate) enum IndexedLaneSnapshot {
    Exact(Vec<Annotation>),
    Activity(Vec<MipmapRecord>),
    Error,
}

#[derive(Clone)]
pub(crate) struct IndexedLaneQuery {
    store: IndexedAnnotationStore,
}

pub(crate) struct IndexedLaneWriter {
    name: String,
    writer: Option<IndexedAnnotationWriter>,
}

pub(crate) fn indexed_lane(
    name: &str,
    config: LiveStoreConfig,
) -> Result<(IndexedLaneWriter, IndexedLaneQuery), String> {
    if let Some(persistent) = config.persistence.as_ref() {
        match IndexedAnnotationStore::open_persistent(persistent) {
            Ok(Some(store)) => {
                return Ok((
                    IndexedLaneWriter {
                        name: name.to_owned(),
                        writer: None,
                    },
                    IndexedLaneQuery { store },
                ));
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(
                lane = %name,
                %error,
                "invalid persistent derived-data cache; rebuilding"
            ),
        }
    }

    IndexedAnnotationWriter::create(config)
        .map(|(writer, store)| {
            (
                IndexedLaneWriter {
                    name: name.to_owned(),
                    writer: Some(writer),
                },
                IndexedLaneQuery { store },
            )
        })
        .map_err(|error| error.to_string())
}

impl IndexedLaneWriter {
    pub(crate) fn append(&mut self, words: &[Word]) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        if let Err(error) = writer.append_batch(words) {
            tracing::warn!(
                lane = %self.name,
                %error,
                "indexed derived-data lane failed; disabling further appends"
            );
            self.writer = None;
        }
    }

    pub(crate) fn finish(&mut self) {
        let Some(mut writer) = self.writer.take() else {
            return;
        };
        if let Err(error) = writer.finish() {
            tracing::warn!(lane = %self.name, %error, "could not finish indexed derived-data lane");
        }
    }
}

impl IndexedLaneQuery {
    pub(crate) fn generation(&self) -> u64 {
        self.store.metadata().generation
    }

    pub(crate) fn snapshot(
        &self,
        start_time_ns: u64,
        end_time_ns: u64,
        max_items: usize,
    ) -> IndexedLaneSnapshot {
        let max_items = max_items.max(1);
        let available_items = self
            .store
            .metadata()
            .total_word_count
            .try_into()
            .unwrap_or(usize::MAX);
        let target_buckets = max_items.min(available_items.max(1));
        let Ok(buckets) =
            self.store
                .coarse_presence_window(start_time_ns, end_time_ns, target_buckets)
        else {
            return IndexedLaneSnapshot::Error;
        };
        let count = buckets
            .iter()
            .map(|bucket| bucket.word_count)
            .fold(0_u64, u64::saturating_add);
        if count > max_items as u64 {
            return IndexedLaneSnapshot::Activity(
                buckets
                    .into_iter()
                    .map(|bucket| MipmapRecord {
                        start_ns: bucket.start_ns,
                        end_ns: bucket.end_ns,
                        count: bucket.word_count.min(u64::from(u32::MAX)) as u32,
                        level_hint: None,
                    })
                    .collect(),
            );
        }
        match self
            .store
            .exact_window(start_time_ns, end_time_ns, target_buckets)
        {
            Ok(window) if window.complete => IndexedLaneSnapshot::Exact(window.annotations),
            Ok(_) => IndexedLaneSnapshot::Activity(
                buckets
                    .into_iter()
                    .map(|bucket| MipmapRecord {
                        start_ns: bucket.start_ns,
                        end_ns: bucket.end_ns,
                        count: bucket.word_count.min(u64::from(u32::MAX)) as u32,
                        level_hint: None,
                    })
                    .collect(),
            ),
            Err(_) => IndexedLaneSnapshot::Error,
        }
    }

    pub(crate) fn latest_word_at_or_before(&self, timestamp_ns: u64) -> Option<Word> {
        self.store
            .latest_word_at_or_before(timestamp_ns)
            .ok()
            .flatten()
    }

    pub(crate) fn nearest_time_boundary(
        &self,
        timestamp_ns: u64,
        max_distance_ns: u64,
    ) -> Option<u64> {
        self.store
            .nearest_boundary(timestamp_ns, max_distance_ns)
            .ok()
            .flatten()
    }

    pub(crate) fn timeline_extent_end_ns(&self) -> Option<u64> {
        self.store.metadata().extent_end_ns
    }

    pub(crate) fn is_live(&self) -> bool {
        self.store.metadata().is_live
    }

    pub(crate) fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        let metadata = self.store.snapshot().metadata;
        CollectedLaneStorageSnapshot {
            backing: if metadata.persistent_cache {
                CollectedLaneStorageBacking::PersistentCache
            } else {
                CollectedLaneStorageBacking::Indexed
            },
            retained_items: Some(
                metadata
                    .committed_word_count
                    .saturating_add(metadata.hot_tail_word_count as u64),
            ),
            resident_bytes: Some(
                (metadata.hot_tail_word_count * std::mem::size_of::<Word>()) as u64,
            ),
            stored_bytes: Some(metadata.committed_data_len),
            index_items: Some(metadata.committed_block_count as u64),
            index_bytes: None,
            live: metadata.status == StoreStatus::Live,
        }
    }
}
