use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use super::catalog::DerivedLanes;
use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::storage::in_memory_storage_snapshot;
use crate::derived_index::{ChunkedMipmap, LaneFold, MipmapRecord};
use crate::derived_word_store::{
    AnnotationQuery, AnnotationStoreBackend, AnnotationStoreMetadata, AnnotationStoreWriterBackend,
    IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig, LiveStoreMetadata,
    StoreStatus, WordPresenceBucket,
};
use crate::errors::WorkResult;
use crate::events::{Annotation, Word};
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadRegistry,
};
use crate::ports::{InputPort, PortDirection, PortSchema};

#[derive(Clone)]
pub struct IndexedAnnotationLane {
    query: Arc<dyn AnnotationQuery>,
    store: IndexedAnnotationStore,
}

/// Immutable bounded result of a built-in word-lane query.
#[derive(Clone, Debug)]
pub enum WordLaneSnapshot {
    Exact {
        annotations: Vec<Annotation>,
        last_timestamp_ns: Option<u64>,
        display_format: Option<String>,
    },
    Presence(Vec<WordPresenceBucket>),
    Activity,
    Error,
}

pub(crate) struct InMemoryWordLaneStorage {
    pub(crate) annotations: Vec<Annotation>,
    pub(crate) summary: ChunkedMipmap<Annotation, AnnotationFold>,
}

pub(crate) enum WordLaneStorage {
    InMemory(InMemoryWordLaneStorage),
    Indexed(IndexedAnnotationLane),
}

/// Adapter-owned retained query for the built-in word payload.
///
/// Generic subscribers use [`CollectedLaneQuery`]. Concrete diagnostics such
/// as the decoder benchmark may additionally inspect the indexed store owned
/// by this adapter.
pub struct CollectedWordLaneQuery {
    storage: Arc<RwLock<WordLaneStorage>>,
    display_format: Option<String>,
}

impl CollectedWordLaneQuery {
    pub fn indexed_lane(&self) -> Option<IndexedAnnotationLane> {
        let storage = self.storage.read().unwrap();
        let WordLaneStorage::Indexed(indexed) = &*storage else {
            return None;
        };
        Some(indexed.clone())
    }

    #[cfg(test)]
    pub(crate) fn in_memory_for_test(storage: InMemoryWordLaneStorage) -> Self {
        Self {
            storage: Arc::new(RwLock::new(WordLaneStorage::InMemory(storage))),
            display_format: None,
        }
    }

    pub(crate) fn snapshot(&self, request: CollectedLaneSnapshotRequest) -> WordLaneSnapshot {
        enum Source {
            InMemory(WordLaneSnapshot),
            Indexed {
                query: Arc<dyn AnnotationQuery>,
                display_format: Option<String>,
            },
        }

        let source = {
            let storage = self.storage.read().unwrap();
            match &*storage {
                WordLaneStorage::InMemory(storage) => {
                    let first = storage
                        .annotations
                        .partition_point(|annotation| annotation.end_ns < request.start_time_ns);
                    let last = storage
                        .annotations
                        .partition_point(|annotation| annotation.start_ns <= request.end_time_ns);
                    let visible = &storage.annotations[first..last];
                    if visible.len() > request.max_items {
                        Source::InMemory(WordLaneSnapshot::Activity)
                    } else {
                        Source::InMemory(WordLaneSnapshot::Exact {
                            annotations: visible.to_vec(),
                            last_timestamp_ns: storage
                                .annotations
                                .last()
                                .map(|annotation| annotation.start_ns),
                            display_format: self.display_format.clone(),
                        })
                    }
                }
                WordLaneStorage::Indexed(indexed) => Source::Indexed {
                    query: Arc::clone(indexed.query()),
                    display_format: self.display_format.clone(),
                },
            }
        };

        match source {
            Source::InMemory(snapshot) => snapshot,
            Source::Indexed {
                query,
                display_format,
            } => {
                let target_points = request.max_items.max(1);
                let Ok(buckets) = query.coarse_presence_window(
                    request.start_time_ns,
                    request.end_time_ns,
                    target_points,
                ) else {
                    return WordLaneSnapshot::Error;
                };
                let count = buckets
                    .iter()
                    .map(|bucket| bucket.word_count)
                    .fold(0u64, u64::saturating_add);
                if count > request.max_items as u64 {
                    return WordLaneSnapshot::Presence(buckets);
                }
                match query.exact_window(
                    request.start_time_ns,
                    request.end_time_ns,
                    request.max_items.max(1),
                ) {
                    Ok(window) if window.complete => WordLaneSnapshot::Exact {
                        annotations: window.annotations,
                        last_timestamp_ns: query.metadata().last_timestamp_ns,
                        display_format,
                    },
                    Ok(_) => WordLaneSnapshot::Presence(buckets),
                    Err(_) => WordLaneSnapshot::Error,
                }
            }
        }
    }
}

impl CollectedLaneQuery for CollectedWordLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(
            self.snapshot(request),
        )))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        let indexed_query = {
            let storage = self.storage.read().unwrap();
            match &*storage {
                WordLaneStorage::InMemory(storage) => {
                    return nearest_annotation_boundary(
                        &storage.annotations,
                        timestamp_ns,
                        max_distance_ns,
                    );
                }
                WordLaneStorage::Indexed(indexed) => Arc::clone(indexed.query()),
            }
        };

        indexed_query
            .nearest_boundary(timestamp_ns, max_distance_ns)
            .ok()
            .flatten()
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        let storage = self.storage.read().unwrap();
        match &*storage {
            WordLaneStorage::InMemory(storage) => storage
                .annotations
                .last()
                .map(|annotation| annotation.end_ns.max(annotation.start_ns)),
            WordLaneStorage::Indexed(indexed) => indexed.metadata().extent_end_ns,
        }
    }

    fn is_live(&self) -> bool {
        let storage = self.storage.read().unwrap();
        matches!(&*storage, WordLaneStorage::Indexed(indexed) if indexed.status() == StoreStatus::Live)
    }

    fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        let storage = self.storage.read().unwrap();
        match &*storage {
            WordLaneStorage::InMemory(storage) => Some(CollectedLaneTableMetadata {
                generation: storage
                    .annotations
                    .last()
                    .map_or(0, |annotation| annotation.end_ns),
                total_rows: storage.annotations.len() as u64,
            }),
            WordLaneStorage::Indexed(indexed) => {
                let metadata = indexed.metadata();
                Some(CollectedLaneTableMetadata {
                    generation: metadata.generation,
                    total_rows: metadata.total_word_count,
                })
            }
        }
    }

    fn table_snapshot(&self, max_rows: usize) -> Option<CollectedLaneTableSnapshot> {
        enum Source {
            InMemory {
                rows: Vec<CollectedLaneTableRow>,
                complete: bool,
                format_hint: Option<String>,
            },
            Indexed {
                query: Arc<dyn AnnotationQuery>,
                format_hint: Option<String>,
            },
        }

        let source = {
            let storage = self.storage.read().unwrap();
            match &*storage {
                WordLaneStorage::InMemory(storage) => Source::InMemory {
                    rows: storage
                        .annotations
                        .iter()
                        .take(max_rows)
                        .map(annotation_table_row)
                        .collect(),
                    complete: storage.annotations.len() <= max_rows,
                    format_hint: self.display_format.clone(),
                },
                WordLaneStorage::Indexed(indexed) => Source::Indexed {
                    query: Arc::clone(indexed.query()),
                    format_hint: self.display_format.clone(),
                },
            }
        };

        match source {
            Source::InMemory {
                rows,
                complete,
                format_hint,
            } => Some(CollectedLaneTableSnapshot {
                rows,
                complete,
                format_hint,
            }),
            Source::Indexed { query, format_hint } => {
                let metadata = query.metadata();
                let window = query
                    .exact_window(
                        metadata.first_timestamp_ns.unwrap_or(0),
                        metadata.extent_end_ns.unwrap_or(u64::MAX),
                        max_rows,
                    )
                    .ok()?;
                Some(CollectedLaneTableSnapshot {
                    rows: window
                        .annotations
                        .iter()
                        .map(annotation_table_row)
                        .collect(),
                    complete: window.complete,
                    format_hint,
                })
            }
        }
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        let storage = self.storage.read().unwrap();
        match &*storage {
            WordLaneStorage::InMemory(storage) => {
                let payload_bytes = storage
                    .annotations
                    .iter()
                    .map(annotation_payload_bytes)
                    .sum::<usize>();
                let mut snapshot = in_memory_storage_snapshot::<Annotation>(
                    storage.annotations.len(),
                    storage.summary.resident_records(),
                );
                snapshot.resident_bytes = snapshot
                    .resident_bytes
                    .map(|bytes| bytes.saturating_add(payload_bytes as u64));
                snapshot
            }
            WordLaneStorage::Indexed(indexed) => {
                let metadata = indexed.storage_metadata();
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
                    resident_bytes: Some((metadata.hot_tail_word_count * size_of::<Word>()) as u64),
                    stored_bytes: Some(metadata.committed_data_len),
                    index_items: Some(metadata.committed_block_count as u64),
                    index_bytes: None,
                    live: metadata.status == StoreStatus::Live,
                }
            }
        }
    }
}

fn annotation_payload_bytes(annotation: &Annotation) -> usize {
    match &annotation.payload {
        Some(crate::events::WordPayload::Bytes(bytes)) => bytes.len(),
        Some(crate::events::WordPayload::Text(text)) => text.len(),
        None => 0,
    }
}

fn annotation_table_row(annotation: &Annotation) -> CollectedLaneTableRow {
    CollectedLaneTableRow {
        start_time_ns: annotation.start_ns,
        end_time_ns: annotation.end_ns,
        value: annotation.value,
        payload: annotation.payload.clone(),
    }
}

fn nearest_annotation_boundary(
    annotations: &[Annotation],
    timestamp_ns: u64,
    max_distance_ns: u64,
) -> Option<u64> {
    let index = annotations.partition_point(|annotation| annotation.start_ns <= timestamp_ns);
    let first = index.saturating_sub(2);
    let last = (index + 2).min(annotations.len());

    annotations[first..last]
        .iter()
        .enumerate()
        .flat_map(|(offset, annotation)| {
            let annotation_index = first + offset;
            let previous_duration_ns = annotation_index.checked_sub(1).map(|previous_index| {
                let previous = &annotations[previous_index];
                previous.end_ns.saturating_sub(previous.start_ns)
            });
            let end_ns = annotation_display_end(
                annotation,
                annotation_index == annotations.len() - 1,
                previous_duration_ns,
            );
            [annotation.start_ns, end_ns]
        })
        .filter(|candidate| candidate.abs_diff(timestamp_ns) <= max_distance_ns)
        .min_by_key(|candidate| candidate.abs_diff(timestamp_ns))
}

fn annotation_display_end(
    annotation: &Annotation,
    is_last_ever: bool,
    previous_duration_ns: Option<u64>,
) -> u64 {
    if is_last_ever && annotation.end_ns == annotation.start_ns {
        annotation.start_ns.saturating_add(
            previous_duration_ns
                .unwrap_or(crate::events::MAX_ANNOTATION_NS)
                .min(crate::events::MAX_ANNOTATION_NS),
        )
    } else {
        annotation.end_ns.max(annotation.start_ns)
    }
}

impl IndexedAnnotationLane {
    pub fn from_store(store: IndexedAnnotationStore) -> Self {
        Self {
            query: Arc::new(store.clone()),
            store,
        }
    }

    pub fn metadata(&self) -> AnnotationStoreMetadata {
        self.query.metadata()
    }

    pub fn query(&self) -> &Arc<dyn AnnotationQuery> {
        &self.query
    }

    pub fn status(&self) -> StoreStatus {
        AnnotationStoreBackend::snapshot(&self.store)
            .metadata
            .status
    }

    pub fn storage_metadata(&self) -> LiveStoreMetadata {
        AnnotationStoreBackend::snapshot(&self.store).metadata
    }

    /// Returns the platform-neutral indexed store handle. Native-only
    /// diagnostics remain methods of the native store implementation rather
    /// than becoming capabilities of a generic viewer lane.
    pub fn store(&self) -> IndexedAnnotationStore {
        self.store.clone()
    }
}

impl std::fmt::Debug for IndexedAnnotationLane {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndexedAnnotationLane")
            .field("metadata", &self.metadata())
            .field("status", &self.status())
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnnotationFold;
impl LaneFold<Annotation> for AnnotationFold {
    fn leaf(entry: &Annotation) -> MipmapRecord {
        MipmapRecord {
            start_ns: entry.start_ns,
            end_ns: entry.end_ns,
            count: 1,
            level_hint: None,
        }
    }
    fn combine(records: &[MipmapRecord]) -> MipmapRecord {
        MipmapRecord {
            start_ns: records[0].start_ns,
            // Not necessarily the last record in append order — boxes can,
            // in principle, close later than a subsequent one starts.
            end_ns: records.iter().map(|record| record.end_ns).max().unwrap(),
            count: records.iter().map(|record| record.count).sum(),
            level_hint: None,
        }
    }
}

/// Adapter-specific construction options for the built-in word payload.
#[derive(Clone)]
pub struct CollectedWordLaneOptions {
    indexed: bool,
    store_config: LiveStoreConfig,
    display_format: Option<String>,
}

impl Default for CollectedWordLaneOptions {
    fn default() -> Self {
        Self {
            indexed: true,
            store_config: LiveStoreConfig::default(),
            display_format: None,
        }
    }
}

impl CollectedWordLaneOptions {
    pub fn new(store_config: LiveStoreConfig, display_format: Option<String>) -> Self {
        Self {
            indexed: true,
            store_config,
            display_format,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_indexed_for_test(&mut self, indexed: bool) {
        self.indexed = indexed;
    }

    #[cfg(test)]
    pub(crate) fn set_store_config_for_test(&mut self, store_config: LiveStoreConfig) {
        self.store_config = store_config;
    }

    #[cfg(test)]
    pub(crate) fn in_memory_for_test() -> Self {
        Self {
            indexed: false,
            ..Self::default()
        }
    }
}

/// Typed append state for the built-in word payload. It owns the indexed
/// writer and fallback in-memory storage policy independently of the other
/// built-in payload adapters.
struct WordLane {
    name: String,
    storage: Arc<RwLock<WordLaneStorage>>,
    buffer: VecDeque<Word>,
    eos: bool,
    writer: Option<IndexedAnnotationWriter>,
    retention: DerivedDataRetention,
}

impl WordLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let options = request
            .options::<CollectedWordLaneOptions>()
            .cloned()
            .unwrap_or_default();
        let lane =
            Self::with_options_inner(request.name().to_owned(), request.retention(), options);
        request.publish_query(Arc::new(CollectedWordLaneQuery {
            storage: Arc::clone(&lane.storage),
            display_format: request
                .options::<CollectedWordLaneOptions>()
                .and_then(|options| options.display_format.clone()),
        }));
        lane
    }

    fn with_options_inner(
        name: String,
        retention: DerivedDataRetention,
        options: CollectedWordLaneOptions,
    ) -> Self {
        if options.indexed {
            if let Some(persistent) = options.store_config.persistence.as_ref() {
                match IndexedAnnotationStore::open_persistent(persistent) {
                    Ok(Some(indexed_store)) => {
                        return Self {
                            name,
                            storage: Arc::new(RwLock::new(WordLaneStorage::Indexed(
                                IndexedAnnotationLane::from_store(indexed_store),
                            ))),
                            buffer: VecDeque::new(),
                            eos: false,
                            writer: None,
                            retention,
                        };
                    }
                    Ok(None) => {}
                    Err(error) => tracing::warn!(
                        lane = %name,
                        %error,
                        "invalid persistent derived-data cache; rebuilding"
                    ),
                }
            }
            match IndexedAnnotationWriter::create(options.store_config) {
                Ok((writer, indexed_store)) => {
                    return Self {
                        name,
                        storage: Arc::new(RwLock::new(WordLaneStorage::Indexed(
                            IndexedAnnotationLane::from_store(indexed_store),
                        ))),
                        buffer: VecDeque::new(),
                        eos: false,
                        writer: Some(writer),
                        retention,
                    };
                }
                Err(error) => tracing::warn!(
                    lane = %name,
                    %error,
                    "could not create indexed derived-data word lane; using in-memory storage"
                ),
            }
        }

        Self {
            name,
            storage: Arc::new(RwLock::new(WordLaneStorage::InMemory(
                InMemoryWordLaneStorage {
                    annotations: Vec::new(),
                    summary: ChunkedMipmap::new(),
                },
            ))),
            buffer: VecDeque::new(),
            eos: false,
            writer: None,
            retention,
        }
    }
}

impl CollectedLaneIngestor for WordLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<Word>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<Word>(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.eos = true,
            }
        } else {
            self.eos = true;
        }
        let batch_len = batch.len();
        if !batch.is_empty() {
            if let Some(writer) = self.writer.as_mut()
                && let Err(error) = AnnotationStoreWriterBackend::append_batch(writer, &batch)
            {
                tracing::warn!(lane = %self.name, %error, "indexed derived-data word lane failed; disabling further appends");
                self.writer = None;
            }
            if let WordLaneStorage::InMemory(storage) = &mut *self.storage.write().unwrap() {
                append_words_to_in_memory_storage(storage, &batch, self.retention);
            }
        }
        if self.eos
            && let Some(mut writer) = self.writer.take()
            && let Err(error) = AnnotationStoreWriterBackend::finish(&mut writer)
        {
            tracing::warn!(lane = %self.name, %error, "could not finish indexed derived-data word lane");
        }
        Ok(batch_len)
    }

    fn is_finished(&self) -> bool {
        self.eos
    }
}

pub(crate) fn append_words_to_in_memory_storage(
    storage: &mut InMemoryWordLaneStorage,
    words: &[Word],
    retention: DerivedDataRetention,
) {
    for word in words {
        let previous_start_ns = storage
            .annotations
            .len()
            .checked_sub(2)
            .map(|index| storage.annotations[index].start_ns);
        if let Some(previous) = storage.annotations.last_mut()
            && previous.end_ns == previous.start_ns
        {
            previous.end_ns = crate::events::instantaneous_word_end_ns(
                previous_start_ns,
                previous.start_ns,
                word.timestamp_ns,
            );
            storage.summary.push(previous);
        }
        let annotation = Annotation {
            start_ns: word.timestamp_ns,
            end_ns: word.timestamp_ns + word.duration_ns,
            value: word.value,
            payload: word.payload.clone(),
        };
        if word.duration_ns > 0 {
            storage.summary.push(&annotation);
        }
        storage.annotations.push(annotation);
    }
    if let Some(target) = retention.trim_target(storage.annotations.len()) {
        let excess = storage.annotations.len() - target;
        storage.annotations.drain(..excess);
    }
}

/// Creates the built-in retained word-lane adapter for non-graph callers
/// such as benchmark tools. Graph compilation obtains the same adapter from
/// [`PayloadRegistry`].
pub fn built_in_word_lane_ingestor(
    name: impl Into<String>,
    lanes: DerivedLanes,
    retention: DerivedDataRetention,
    options: CollectedWordLaneOptions,
) -> Box<dyn CollectedLaneIngestor> {
    let mut payloads = PayloadRegistry::new();
    payloads
        .register::<Word>("org.logicconduit.word/v1")
        .expect("built-in word payload identity must be valid");
    payloads
        .register_adapter::<Word>(word_payload_adapter())
        .expect("built-in word payload adapter must be valid");
    let payload = payloads
        .descriptor::<Word>()
        .expect("built-in word payload must be registered")
        .clone();
    Box::new(WordLane::new(
        CollectedLaneRequest::new(name, 0, lanes, payload, retention).with_options(options),
    ))
}

struct WordPayloadAdapter;

impl PayloadAdapter for WordPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(WordLane::new(request)))
    }
}

pub fn word_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(WordPayloadAdapter)
}
