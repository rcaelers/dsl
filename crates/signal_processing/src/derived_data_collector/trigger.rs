use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::indexed::{IndexedLaneQuery, IndexedLaneSnapshot, IndexedLaneWriter, indexed_lane};
use super::storage::in_memory_storage_snapshot;
use crate::Word;
use crate::derived_index::{AppendOnlyMipmap, LaneFold, MipmapRecord};
use crate::errors::WorkResult;
use crate::events::Trigger;
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
};
use crate::ports::{InputPort, PortDirection, PortSchema};

/// Immutable bounded result of a built-in trigger-marker lane query.
#[derive(Clone, Debug)]
pub enum TriggerLaneSnapshot {
    /// Exact trigger timestamps in the requested visible window.
    Exact(Vec<u64>),
    /// Bounded activity records for a dense visible window.
    Activity(Vec<MipmapRecord>),
}

#[derive(Default)]
pub(crate) struct TriggerLaneStorage {
    pub(crate) timestamps: Vec<u64>,
    pub(crate) summary: AppendOnlyMipmap<u64, MarkerFold>,
    pub(crate) generation: u64,
}

pub(crate) struct TriggerLaneQuery {
    pub(crate) storage: Arc<RwLock<TriggerLaneStorage>>,
    pub(crate) indexed: Option<IndexedLaneQuery>,
}

impl CollectedLaneQuery for TriggerLaneQuery {
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
        let snapshot = if let Some(indexed) = &self.indexed {
            match indexed.snapshot(
                request.start_time_ns,
                request.end_time_ns,
                request.max_items,
            ) {
                IndexedLaneSnapshot::Exact(annotations) => TriggerLaneSnapshot::Exact(
                    annotations
                        .into_iter()
                        .map(|annotation| annotation.start_ns)
                        .collect(),
                ),
                IndexedLaneSnapshot::Activity(records) => TriggerLaneSnapshot::Activity(records),
                IndexedLaneSnapshot::Error => return None,
            }
        } else {
            let storage = self.storage.try_read().ok()?;
            let first = storage
                .timestamps
                .partition_point(|timestamp| *timestamp < request.start_time_ns);
            let last = storage
                .timestamps
                .partition_point(|timestamp| *timestamp <= request.end_time_ns);
            if last - first <= request.max_items {
                TriggerLaneSnapshot::Exact(storage.timestamps[first..last].to_vec())
            } else {
                TriggerLaneSnapshot::Activity(storage.summary.sampled_window(
                    request.start_time_ns,
                    request.end_time_ns,
                    request.max_items,
                ))
            }
        };
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(snapshot)))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        if let Some(indexed) = &self.indexed {
            return indexed.nearest_time_boundary(timestamp_ns, max_distance_ns);
        }
        let storage = self.storage.try_read().ok()?;
        let index = storage
            .timestamps
            .partition_point(|marker| *marker <= timestamp_ns);
        storage.timestamps[index.saturating_sub(1)..(index + 1).min(storage.timestamps.len())]
            .iter()
            .copied()
            .filter(|candidate| candidate.abs_diff(timestamp_ns) <= max_distance_ns)
            .min_by_key(|candidate| candidate.abs_diff(timestamp_ns))
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        if let Some(indexed) = &self.indexed {
            return indexed.timeline_extent_end_ns();
        }
        self.storage.try_read().ok()?.timestamps.last().copied()
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        if let Some(indexed) = &self.indexed {
            return indexed.storage_snapshot();
        }
        let Ok(storage) = self.storage.try_read() else {
            return CollectedLaneStorageSnapshot::adapter_managed(true);
        };
        in_memory_storage_snapshot::<u64>(
            storage.timestamps.len(),
            storage.summary.resident_records(),
        )
    }

    fn is_live(&self) -> bool {
        self.indexed.as_ref().is_some_and(IndexedLaneQuery::is_live)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct MarkerFold;
impl LaneFold<u64> for MarkerFold {
    fn leaf(entry: &u64) -> MipmapRecord {
        MipmapRecord {
            start_ns: *entry,
            end_ns: *entry,
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

/// Typed append state for the built-in trigger payload.
struct TriggerLane {
    storage: Arc<RwLock<TriggerLaneStorage>>,
    buffer: VecDeque<Trigger>,
    eos: bool,
    retention: DerivedDataRetention,
    indexed: Option<IndexedLaneWriter>,
}

impl TriggerLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let storage = Arc::new(RwLock::new(TriggerLaneStorage::default()));
        let (indexed, indexed_query) = request.indexed_store().cloned().map_or(
            (None, None),
            |config| match indexed_lane(request.name(), config) {
                Ok((writer, query)) => (Some(writer), Some(query)),
                Err(error) => {
                    tracing::warn!(lane = request.name(), %error, "could not create indexed trigger lane; using memory");
                    (None, None)
                }
            },
        );
        request.publish_query(Arc::new(TriggerLaneQuery {
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

impl CollectedLaneIngestor for TriggerLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<Trigger>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<Trigger>(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.eos = true,
            }
        } else {
            self.eos = true;
        }
        let batch_len = batch.len();
        if !batch.is_empty() {
            let timestamps = batch
                .iter()
                .map(|trigger| trigger.timestamp_ns)
                .collect::<Vec<_>>();
            if let Some(indexed) = &mut self.indexed {
                let words = timestamps
                    .iter()
                    .map(|timestamp_ns| Word::new(0, *timestamp_ns))
                    .collect::<Vec<_>>();
                indexed.append(&words);
            } else {
                let mut storage = self.storage.write().unwrap();
                for timestamp_ns in &timestamps {
                    storage.summary.push(timestamp_ns);
                }
                storage.timestamps.extend(timestamps.iter().copied());
                if let Some(target) = self.retention.trim_target(storage.timestamps.len()) {
                    let excess = storage.timestamps.len() - target;
                    storage.timestamps.drain(..excess);
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

struct TriggerPayloadAdapter;

impl PayloadAdapter for TriggerPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(TriggerLane::new(request)))
    }
}

pub fn trigger_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(TriggerPayloadAdapter)
}
