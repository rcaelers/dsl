use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use signal_capture::Sample;
use signal_runtime::{InputPort, PortDirection, PortSchema, WorkResult};

use super::collector::{DRAIN_BATCH_SIZE, DerivedDataRetention};
use super::indexed::{IndexedLaneQuery, IndexedLaneSnapshot, IndexedLaneWriter, indexed_lane};
use super::storage::in_memory_storage_snapshot;
use crate::Word;
use crate::derived_index::{AppendOnlyMipmap, LaneFold, MipmapRecord};
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
};

/// Immutable bounded result of a built-in digital-lane query.
#[derive(Clone, Debug)]
pub enum DigitalLaneSnapshot {
    /// Exact level transitions in the requested visible window.
    Exact { samples: Vec<Sample>, initial: bool },
    /// Bounded summary records for a dense visible window.
    Activity {
        records: Vec<MipmapRecord>,
        initial: bool,
    },
}

#[derive(Default)]
pub(crate) struct DigitalLaneStorage {
    pub(crate) samples: Vec<Sample>,
    pub(crate) summary: AppendOnlyMipmap<Sample, DigitalFold>,
    pub(crate) generation: u64,
}

pub(crate) struct DigitalLaneQuery {
    pub(crate) storage: Arc<RwLock<DigitalLaneStorage>>,
    pub(crate) indexed: Option<IndexedLaneQuery>,
}

impl CollectedLaneQuery for DigitalLaneQuery {
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
            let mut initial = indexed
                .latest_word_at_or_before(request.start_time_ns.saturating_sub(1))
                .is_some_and(|word| word.value != 0);
            match indexed.snapshot(
                request.start_time_ns,
                request.end_time_ns,
                request.max_items,
            ) {
                IndexedLaneSnapshot::Exact(annotations) => {
                    let samples = annotations
                        .into_iter()
                        .filter_map(|annotation| {
                            if annotation.start_ns < request.start_time_ns {
                                initial = annotation.value != 0;
                                None
                            } else {
                                Some(Sample::new(annotation.value != 0, annotation.start_ns))
                            }
                        })
                        .collect();
                    DigitalLaneSnapshot::Exact { samples, initial }
                }
                IndexedLaneSnapshot::Activity(records) => {
                    DigitalLaneSnapshot::Activity { records, initial }
                }
                IndexedLaneSnapshot::Error => return None,
            }
        } else {
            let storage = self.storage.try_read().ok()?;
            let first = storage
                .samples
                .partition_point(|sample| sample.start_time_ns < request.start_time_ns);
            let last = storage
                .samples
                .partition_point(|sample| sample.start_time_ns <= request.end_time_ns);
            let initial = first
                .checked_sub(1)
                .and_then(|index| storage.samples.get(index))
                .is_some_and(|sample| sample.value);
            if last - first <= request.max_items {
                DigitalLaneSnapshot::Exact {
                    samples: storage.samples[first..last].to_vec(),
                    initial,
                }
            } else {
                DigitalLaneSnapshot::Activity {
                    records: storage.summary.sampled_window(
                        request.start_time_ns,
                        request.end_time_ns,
                        request.max_items,
                    ),
                    initial,
                }
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
            .samples
            .partition_point(|sample| sample.start_time_ns <= timestamp_ns);
        storage.samples[index.saturating_sub(1)..(index + 1).min(storage.samples.len())]
            .iter()
            .map(|sample| sample.start_time_ns)
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
            .samples
            .last()
            .map(|sample| sample.start_time_ns)
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        if let Some(indexed) = &self.indexed {
            return indexed.storage_snapshot();
        }
        let Ok(storage) = self.storage.try_read() else {
            return CollectedLaneStorageSnapshot::adapter_managed(true);
        };
        in_memory_storage_snapshot::<Sample>(
            storage.samples.len(),
            storage.summary.resident_records(),
        )
    }

    fn is_live(&self) -> bool {
        self.indexed.as_ref().is_some_and(IndexedLaneQuery::is_live)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DigitalFold;
impl LaneFold<Sample> for DigitalFold {
    fn leaf(entry: &Sample) -> MipmapRecord {
        MipmapRecord {
            start_ns: entry.start_time_ns,
            end_ns: entry.start_time_ns,
            count: 1,
            level_hint: Some((entry.value, entry.value)),
        }
    }
    fn combine(records: &[MipmapRecord]) -> MipmapRecord {
        let first = records[0];
        let last = records[records.len() - 1];
        MipmapRecord {
            start_ns: first.start_ns,
            end_ns: last.end_ns,
            count: records.iter().map(|record| record.count).sum(),
            level_hint: match (first.level_hint, last.level_hint) {
                (Some((first, _)), Some((_, last))) => Some((first, last)),
                _ => None,
            },
        }
    }
}

/// Typed append state for the built-in digital payload.
struct DigitalLane {
    storage: Arc<RwLock<DigitalLaneStorage>>,
    buffer: VecDeque<Sample>,
    eos: bool,
    retention: DerivedDataRetention,
    indexed: Option<IndexedLaneWriter>,
}

impl DigitalLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let storage = Arc::new(RwLock::new(DigitalLaneStorage::default()));
        let (indexed, indexed_query) = request.indexed_store().cloned().map_or(
            (None, None),
            |config| {
                match indexed_lane(
                    request.name(),
                    config,
                    request.decoded_block_cache().clone(),
                ) {
                Ok((writer, query)) => (Some(writer), Some(query)),
                Err(error) => {
                    tracing::warn!(lane = request.name(), %error, "could not create indexed digital lane; using memory");
                    (None, None)
                }
                }
            },
        );
        request.publish_query(Arc::new(DigitalLaneQuery {
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

impl CollectedLaneIngestor for DigitalLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::state::<Sample>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<Sample>(&mut self.buffer) {
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
                    .map(|sample| Word::new(u64::from(sample.value), sample.start_time_ns))
                    .collect::<Vec<_>>();
                indexed.append(&words);
            } else {
                let mut storage = self.storage.write().unwrap();
                for sample in &batch {
                    storage.summary.push(sample);
                }
                storage.samples.extend(batch.iter().copied());
                if let Some(target) = self.retention.trim_target(storage.samples.len()) {
                    let excess = storage.samples.len() - target;
                    storage.samples.drain(..excess);
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

struct DigitalPayloadAdapter;

impl PayloadAdapter for DigitalPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(DigitalLane::new(request)))
    }
}

/// Returns the payload adapter for built-in digital lanes.
pub fn digital_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(DigitalPayloadAdapter)
}
