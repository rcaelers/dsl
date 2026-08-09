use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use signal_runtime::{InputPort, PortDirection, PortSchema, WorkResult};

use super::catalog::DerivedLanes;
use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::storage::in_memory_storage_snapshot;
use crate::derived_index::{ChunkedMipmap, LaneFold, MipmapRecord};
use crate::derived_word_store::{
    AnnotationQuery, AnnotationStoreBackend, AnnotationStoreMetadata, AnnotationStoreWriterBackend,
    DecodedBlockCacheHandle, IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig,
    LiveStoreMetadata, StoreStatus, WordPresenceBucket,
};
use crate::events::{Annotation, Word};
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadRegistry,
};
use crate::payload_ingestor_construction_error::PayloadIngestorConstructionError;

const WORD_DRAIN_BATCH_SIZE: usize = DRAIN_BATCH_SIZE * 2;

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
    pub(crate) generation: u64,
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
    /// Returns the durable indexed annotation lane when this collector uses one.
    pub fn indexed_lane(&self) -> Option<IndexedAnnotationLane> {
        let storage = self.storage.try_read().ok()?;
        let WordLaneStorage::Indexed(indexed) = &*storage else {
            return None;
        };
        Some(indexed.clone())
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "the collector's in-memory test suite is native-only"
    )]
    pub(crate) fn in_memory_for_test(storage: InMemoryWordLaneStorage) -> Self {
        Self {
            storage: Arc::new(RwLock::new(WordLaneStorage::InMemory(storage))),
            display_format: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn snapshot(&self, request: CollectedLaneSnapshotRequest) -> WordLaneSnapshot {
        self.try_snapshot(request)
            .unwrap_or(WordLaneSnapshot::Error)
    }

    fn try_snapshot(&self, request: CollectedLaneSnapshotRequest) -> Option<WordLaneSnapshot> {
        enum Source {
            InMemory(WordLaneSnapshot),
            Indexed {
                query: Arc<dyn AnnotationQuery>,
                display_format: Option<String>,
            },
        }

        let source = {
            let storage = self.storage.try_read().ok()?;
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
                let metadata = query.metadata();
                let available_items = metadata.total_word_count.try_into().unwrap_or(usize::MAX);
                let target_points = request.max_items.max(1).min(available_items.max(1));
                let Ok(buckets) = query.coarse_presence_window(
                    request.start_time_ns,
                    request.end_time_ns,
                    target_points,
                ) else {
                    return Some(WordLaneSnapshot::Error);
                };
                let count = buckets
                    .iter()
                    .map(|bucket| bucket.word_count)
                    .fold(0u64, u64::saturating_add);
                if count > request.max_items as u64 {
                    return Some(WordLaneSnapshot::Presence(buckets));
                }
                match query.exact_window(request.start_time_ns, request.end_time_ns, target_points)
                {
                    Ok(window) if window.complete => WordLaneSnapshot::Exact {
                        annotations: window.annotations,
                        last_timestamp_ns: metadata.last_timestamp_ns,
                        display_format,
                    },
                    Ok(_) => WordLaneSnapshot::Presence(buckets),
                    Err(_) => WordLaneSnapshot::Error,
                }
            }
        }
        .into()
    }
}

impl CollectedLaneQuery for CollectedWordLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn snapshot_generation(&self) -> Option<u64> {
        let storage = self.storage.try_read().ok()?;
        Some(match &*storage {
            WordLaneStorage::InMemory(storage) => storage.generation,
            WordLaneStorage::Indexed(indexed) => indexed.metadata().generation,
        })
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        self.try_snapshot(request)
            .map(|snapshot| OpaqueCollectedLaneSnapshot::new(Arc::new(snapshot)))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        let indexed_query = {
            let storage = self.storage.try_read().ok()?;
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
        let storage = self.storage.try_read().ok()?;
        match &*storage {
            WordLaneStorage::InMemory(storage) => storage
                .annotations
                .last()
                .map(|annotation| annotation.end_ns.max(annotation.start_ns)),
            WordLaneStorage::Indexed(indexed) => indexed.metadata().extent_end_ns,
        }
    }

    fn is_live(&self) -> bool {
        let Ok(storage) = self.storage.try_read() else {
            return true;
        };
        matches!(&*storage, WordLaneStorage::Indexed(indexed) if indexed.status() == StoreStatus::Live)
    }

    fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        let storage = self.storage.try_read().ok()?;
        match &*storage {
            WordLaneStorage::InMemory(storage) => Some(CollectedLaneTableMetadata {
                generation: storage.generation,
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
            let storage = self.storage.try_read().ok()?;
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
        let Ok(storage) = self.storage.try_read() else {
            return CollectedLaneStorageSnapshot::adapter_managed(true);
        };
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
    /// Wraps an indexed annotation store as a viewer-discoverable lane.
    ///
    /// # Parameters
    /// - `store`: Finished or live store that owns exact word history.
    pub fn from_store(store: IndexedAnnotationStore) -> Self {
        Self {
            query: Arc::new(store.clone()),
            store,
        }
    }

    /// Returns current annotation query metadata.
    pub fn metadata(&self) -> AnnotationStoreMetadata {
        self.query.metadata()
    }

    /// Returns the polymorphic viewer annotation query handle.
    pub fn query(&self) -> &Arc<dyn AnnotationQuery> {
        &self.query
    }

    /// Returns the store lifecycle status.
    pub fn status(&self) -> StoreStatus {
        AnnotationStoreBackend::snapshot(&self.store)
            .metadata
            .status
    }

    /// Returns committed-block and hot-tail storage metadata.
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
    /// Creates indexed word-lane options with an optional display format.
    ///
    /// # Parameters
    /// - `store_config`: Live encoding and persistence configuration.
    /// - `display_format`: Optional adapter-defined display-format identifier.
    pub fn new(store_config: LiveStoreConfig, display_format: Option<String>) -> Self {
        Self {
            indexed: true,
            store_config,
            display_format,
        }
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "the collector's in-memory test suite is native-only"
    )]
    pub(crate) fn set_indexed_for_test(&mut self, indexed: bool) {
        self.indexed = indexed;
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "the collector's in-memory test suite is native-only"
    )]
    pub(crate) fn set_store_config_for_test(&mut self, store_config: LiveStoreConfig) {
        self.store_config = store_config;
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "the collector's in-memory test suite is native-only"
    )]
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
    in_memory: bool,
}

impl WordLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let decoded_block_cache = request.decoded_block_cache().clone();
        let options = request
            .options::<CollectedWordLaneOptions>()
            .cloned()
            .unwrap_or_default();
        let lane = Self::with_options_inner(
            request.name().to_owned(),
            request.retention(),
            options,
            decoded_block_cache,
        );
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
        decoded_block_cache: DecodedBlockCacheHandle,
    ) -> Self {
        if options.indexed {
            if let Some(persistent) = options.store_config.persistence.as_ref() {
                match IndexedAnnotationStore::open_persistent(
                    persistent,
                    decoded_block_cache.clone(),
                ) {
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
                            in_memory: false,
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
            match IndexedAnnotationWriter::create(options.store_config, decoded_block_cache) {
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
                        in_memory: false,
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
                    generation: 0,
                },
            ))),
            buffer: VecDeque::new(),
            eos: false,
            writer: None,
            retention,
            in_memory: true,
        }
    }
}

impl CollectedLaneIngestor for WordLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<Word>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batches = Vec::new();
        let mut batch_len = 0usize;
        if let Some(mut receiver) = input.get::<Word>(&mut self.buffer) {
            while batch_len < WORD_DRAIN_BATCH_SIZE {
                match receiver.try_recv_batch() {
                    Ok(batch) => {
                        batch_len += batch.len();
                        batches.push(batch);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.eos = true;
                        break;
                    }
                }
            }
        } else {
            self.eos = true;
        }
        if !batches.is_empty() {
            if let Some(writer) = self.writer.as_mut()
                && let Err(error) = AnnotationStoreWriterBackend::append_batches(writer, &batches)
            {
                tracing::warn!(lane = %self.name, %error, "indexed derived-data word lane failed; disabling further appends");
                self.writer = None;
            }
            if self.in_memory
                && let WordLaneStorage::InMemory(storage) = &mut *self.storage.write().unwrap()
            {
                for batch in &batches {
                    append_words_to_in_memory_storage(storage, batch, self.retention);
                }
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
    if words.is_empty() {
        return;
    }
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
    storage.generation = storage.generation.wrapping_add(1);
}

/// Creates the built-in retained word-lane adapter for non-graph callers
/// such as benchmark tools. Graph compilation obtains the same adapter from
/// [`PayloadRegistry`].
///
/// # Parameters
/// - `name`: Input consumed by this operation.
/// - `lanes`: Input consumed by this operation.
/// - `retention`: Input consumed by this operation.
/// - `options`: Input consumed by this operation.
/// - `decoded_block_cache`: Cache shared by indexed stores created for this lane.
pub fn built_in_word_lane_ingestor(
    name: impl Into<String>,
    lanes: DerivedLanes,
    retention: DerivedDataRetention,
    options: CollectedWordLaneOptions,
    decoded_block_cache: DecodedBlockCacheHandle,
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
        CollectedLaneRequest::new(name, 0, lanes, payload, retention)
            .with_decoded_block_cache(decoded_block_cache)
            .with_options(options),
    ))
}

struct WordPayloadAdapter;

impl PayloadAdapter for WordPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, PayloadIngestorConstructionError> {
        Ok(Box::new(WordLane::new(request)))
    }
}

/// Returns the payload adapter for built-in decoded-word lanes.
pub fn word_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(WordPayloadAdapter)
}
