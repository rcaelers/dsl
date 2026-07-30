use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::storage::in_memory_storage_snapshot;
use crate::derived_index::{AppendOnlyMipmap, LaneFold, MipmapRecord};
use crate::errors::WorkResult;
use crate::events::NumberSample;
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
};
use crate::ports::{InputPort, PortDirection, PortSchema};

/// Immutable bounded result of a built-in numeric-level lane query.
#[derive(Clone, Debug)]
pub enum NumberLaneSnapshot {
    Exact(Vec<NumberSample>),
    Activity(Vec<MipmapRecord>),
}

#[derive(Default)]
pub(crate) struct NumberLaneStorage {
    pub(crate) values: Vec<NumberSample>,
    pub(crate) summary: AppendOnlyMipmap<NumberSample, NumberFold>,
}

pub(crate) struct NumberLaneQuery {
    pub(crate) storage: Arc<RwLock<NumberLaneStorage>>,
}

impl CollectedLaneQuery for NumberLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        let storage = self.storage.read().unwrap();
        let first = storage
            .values
            .partition_point(|value| value.start_time_ns < request.start_time_ns)
            .saturating_sub(1);
        let last = storage
            .values
            .partition_point(|value| value.start_time_ns <= request.end_time_ns);
        let snapshot = if last.saturating_sub(first) <= request.max_items {
            NumberLaneSnapshot::Exact(storage.values[first..last].to_vec())
        } else {
            NumberLaneSnapshot::Activity(storage.summary.sampled_window(
                request.start_time_ns,
                request.end_time_ns,
                request.max_items,
            ))
        };
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(snapshot)))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        let storage = self.storage.read().unwrap();
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
        self.storage
            .read()
            .unwrap()
            .values
            .last()
            .map(|value| value.start_time_ns)
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        let storage = self.storage.read().unwrap();
        in_memory_storage_snapshot::<NumberSample>(
            storage.values.len(),
            storage.summary.resident_records(),
        )
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct NumberFold;
impl LaneFold<NumberSample> for NumberFold {
    fn leaf(entry: &NumberSample) -> MipmapRecord {
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

/// Typed append state for the built-in numeric-level payload.
struct NumberLane {
    storage: Arc<RwLock<NumberLaneStorage>>,
    buffer: VecDeque<NumberSample>,
    eos: bool,
    retention: DerivedDataRetention,
}

impl NumberLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let storage = Arc::new(RwLock::new(NumberLaneStorage::default()));
        request.publish_query(Arc::new(NumberLaneQuery {
            storage: Arc::clone(&storage),
        }));
        Self {
            storage,
            buffer: VecDeque::new(),
            eos: false,
            retention: request.retention(),
        }
    }
}

impl CollectedLaneIngestor for NumberLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<NumberSample>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<NumberSample>(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.eos = true,
            }
        } else {
            self.eos = true;
        }
        let batch_len = batch.len();
        if !batch.is_empty() {
            let mut storage = self.storage.write().unwrap();
            for sample in &batch {
                storage.summary.push(sample);
            }
            storage.values.extend(batch.iter().copied());
            if let Some(target) = self.retention.trim_target(storage.values.len()) {
                let excess = storage.values.len() - target;
                storage.values.drain(..excess);
            }
        }
        Ok(batch_len)
    }

    fn is_finished(&self) -> bool {
        self.eos
    }
}

struct NumberPayloadAdapter;

impl PayloadAdapter for NumberPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(NumberLane::new(request)))
    }
}

pub fn number_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(NumberPayloadAdapter)
}
