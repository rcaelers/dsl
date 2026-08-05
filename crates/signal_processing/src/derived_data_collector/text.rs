use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use signal_runtime::{InputPort, PortDirection, PortSchema, WorkResult};

use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::indexed::{IndexedLaneQuery, IndexedLaneSnapshot, IndexedLaneWriter, indexed_lane};
use super::storage::in_memory_storage_snapshot;
use crate::derived_index::{AppendOnlyMipmap, LaneFold, MipmapRecord};
use crate::events::TextSample;
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
};
use crate::{Word, WordPayload};

/// Immutable bounded result of a built-in text-level lane query.
#[derive(Clone, Debug)]
pub enum TextLaneSnapshot {
    Exact(Vec<TextSample>),
    Activity(Vec<MipmapRecord>),
}

#[derive(Default)]
pub(crate) struct TextLaneStorage {
    pub(crate) values: Vec<TextSample>,
    pub(crate) summary: AppendOnlyMipmap<TextSample, TextFold>,
    pub(crate) generation: u64,
}

pub(crate) struct TextLaneQuery {
    pub(crate) storage: Arc<RwLock<TextLaneStorage>>,
    pub(crate) indexed: Option<IndexedLaneQuery>,
}

impl CollectedLaneQuery for TextLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn snapshot_generation(&self) -> Option<u64> {
        self.indexed
            .as_ref()
            .map(IndexedLaneQuery::generation)
            .or_else(|| {
                self.storage
                    .try_read()
                    .ok()
                    .map(|storage| storage.generation)
            })
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        if let Some(indexed) = &self.indexed {
            let previous = indexed
                .latest_word_at_or_before(request.start_time_ns.saturating_sub(1))
                .map(text_sample_from_word);
            let snapshot = match indexed.snapshot(
                request.start_time_ns,
                request.end_time_ns,
                request.max_items,
            ) {
                IndexedLaneSnapshot::Exact(annotations) => {
                    let mut values = previous.into_iter().collect::<Vec<_>>();
                    values.extend(annotations.into_iter().map(|annotation| {
                        TextSample::new(
                            annotation
                                .payload
                                .as_ref()
                                .and_then(text_payload)
                                .unwrap_or_default(),
                            annotation.start_ns,
                        )
                    }));
                    TextLaneSnapshot::Exact(values)
                }
                IndexedLaneSnapshot::Activity(records) => TextLaneSnapshot::Activity(records),
                IndexedLaneSnapshot::Error => return None,
            };
            return Some(OpaqueCollectedLaneSnapshot::new(Arc::new(snapshot)));
        }
        let storage = self.storage.try_read().ok()?;
        let first = storage
            .values
            .partition_point(|value| value.start_time_ns < request.start_time_ns)
            .saturating_sub(1);
        let last = storage
            .values
            .partition_point(|value| value.start_time_ns <= request.end_time_ns);
        let snapshot = if last.saturating_sub(first) <= request.max_items {
            TextLaneSnapshot::Exact(storage.values[first..last].to_vec())
        } else {
            TextLaneSnapshot::Activity(storage.summary.sampled_window(
                request.start_time_ns,
                request.end_time_ns,
                request.max_items,
            ))
        };
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(snapshot)))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        if let Some(indexed) = &self.indexed {
            return indexed.nearest_time_boundary(timestamp_ns, max_distance_ns);
        }
        let storage = self.storage.try_read().ok()?;
        let index = storage
            .values
            .partition_point(|value| value.start_time_ns <= timestamp_ns);
        storage.values[index.saturating_sub(1)..(index + 1).min(storage.values.len())]
            .iter()
            .map(|value| value.start_time_ns)
            .filter(|candidate| candidate.abs_diff(timestamp_ns) <= max_distance_ns)
            .min_by_key(|candidate| candidate.abs_diff(timestamp_ns))
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        if let Some(indexed) = &self.indexed {
            return indexed.timeline_extent_end_ns();
        }
        self.storage
            .try_read()
            .ok()?
            .values
            .last()
            .map(|value| value.start_time_ns)
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        if let Some(indexed) = &self.indexed {
            return indexed.storage_snapshot();
        }
        let Ok(storage) = self.storage.try_read() else {
            return CollectedLaneStorageSnapshot::adapter_managed(true);
        };
        let payload_bytes = storage
            .values
            .iter()
            .map(|value| value.value.capacity())
            .sum::<usize>();
        let mut snapshot = in_memory_storage_snapshot::<TextSample>(
            storage.values.len(),
            storage.summary.resident_records(),
        );
        snapshot.resident_bytes = snapshot
            .resident_bytes
            .map(|bytes| bytes.saturating_add(payload_bytes as u64));
        snapshot
    }

    fn is_live(&self) -> bool {
        self.indexed.as_ref().is_some_and(IndexedLaneQuery::is_live)
    }
}

fn text_payload(payload: &WordPayload) -> Option<String> {
    match payload {
        WordPayload::Text(text) => Some(text.to_string()),
        WordPayload::Bytes(bytes) => String::from_utf8(bytes.to_vec()).ok(),
    }
}

fn text_sample_from_word(word: Word) -> TextSample {
    TextSample::new(
        word.payload
            .as_ref()
            .and_then(text_payload)
            .unwrap_or_default(),
        word.timestamp_ns,
    )
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TextFold;
impl LaneFold<TextSample> for TextFold {
    fn leaf(entry: &TextSample) -> MipmapRecord {
        MipmapRecord {
            start_ns: entry.start_time_ns,
            end_ns: entry.start_time_ns,
            count: 1,
            level_hint: None,
        }
    }

    fn combine(records: &[MipmapRecord]) -> MipmapRecord {
        MipmapRecord {
            start_ns: records[0].start_ns,
            end_ns: records[records.len() - 1].end_ns,
            count: records.iter().map(|record| record.count).sum(),
            level_hint: None,
        }
    }
}

/// Typed append state for the built-in text-level payload.
struct TextLane {
    storage: Arc<RwLock<TextLaneStorage>>,
    buffer: VecDeque<TextSample>,
    eos: bool,
    retention: DerivedDataRetention,
    indexed: Option<IndexedLaneWriter>,
}

impl TextLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let storage = Arc::new(RwLock::new(TextLaneStorage::default()));
        let (indexed, indexed_query) = request.indexed_store().cloned().map_or(
            (None, None),
            |config| match indexed_lane(request.name(), config) {
                Ok((writer, query)) => (Some(writer), Some(query)),
                Err(error) => {
                    tracing::warn!(lane = request.name(), %error, "could not create indexed text lane; using memory");
                    (None, None)
                }
            },
        );
        request.publish_query(Arc::new(TextLaneQuery {
            storage: Arc::clone(&storage),
            indexed: indexed_query,
        }));
        Self {
            storage,
            buffer: VecDeque::new(),
            eos: false,
            retention: request.retention(),
            indexed,
        }
    }
}

impl CollectedLaneIngestor for TextLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::state::<TextSample>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<TextSample>(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.eos = true,
            }
        } else {
            self.eos = true;
        }
        let batch_len = batch.len();
        if !batch.is_empty() {
            if let Some(indexed) = &mut self.indexed {
                let words = batch
                    .iter()
                    .map(|sample| Word::text(sample.value.clone(), sample.start_time_ns, 0))
                    .collect::<Vec<_>>();
                indexed.append(&words);
            } else {
                let mut storage = self.storage.write().unwrap();
                for sample in &batch {
                    storage.summary.push(sample);
                }
                storage.values.extend(batch.iter().cloned());
                if let Some(target) = self.retention.trim_target(storage.values.len()) {
                    let excess = storage.values.len() - target;
                    storage.values.drain(..excess);
                }
                storage.generation = storage.generation.wrapping_add(1);
            }
        }
        if self.eos
            && let Some(indexed) = &mut self.indexed
        {
            indexed.finish();
        }
        Ok(batch_len)
    }

    fn is_finished(&self) -> bool {
        self.eos
    }
}

struct TextPayloadAdapter;

impl PayloadAdapter for TextPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(TextLane::new(request)))
    }
}

/// Returns the payload adapter for built-in text lanes.
pub fn text_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(TextPayloadAdapter)
}
