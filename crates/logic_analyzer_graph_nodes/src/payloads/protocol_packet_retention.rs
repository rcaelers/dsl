use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use logic_analyzer_protocol_decoders::types::{ProtocolPacket, ProtocolValue};
use signal_derived::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, DerivedDataRetention,
    OpaqueCollectedLaneSnapshot, PayloadAdapter, PayloadIngestorConstructionError, WordPayload,
};
use signal_runtime::{InputPort, PortDirection, PortSchema, WorkResult};

use super::protocol_packet_presentation::protocol_packet_fallback_label;

const DRAIN_BATCH_SIZE: usize = 1_024;

/// Bounded protocol-packet projection for a visible timeline window.
#[derive(Clone, Debug, PartialEq)]
pub struct ProtocolPacketLaneSnapshot {
    packets: Vec<ProtocolPacket>,
    activity_spans: Vec<(u64, u64)>,
}

impl ProtocolPacketLaneSnapshot {
    /// Returns individually retained packets when the visible window is not dense.
    pub fn packets(&self) -> &[ProtocolPacket] {
        &self.packets
    }

    /// Returns coarse activity spans when the visible packet count exceeds the request limit.
    pub fn activity_spans(&self) -> &[(u64, u64)] {
        &self.activity_spans
    }
}

struct RetainedState {
    packets: Vec<ProtocolPacket>,
    generation: u64,
}

struct ProtocolPacketLaneQuery {
    state: Arc<RwLock<RetainedState>>,
    live: Arc<AtomicBool>,
}

impl CollectedLaneQuery for ProtocolPacketLaneQuery {
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn snapshot_generation(&self) -> Option<u64> {
        Some(self.state.read().unwrap().generation)
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        let state = self.state.read().unwrap();
        let visible = state
            .packets
            .iter()
            .filter(|packet| {
                packet.end_time_ns >= request.start_time_ns
                    && packet.start_time_ns <= request.end_time_ns
            })
            .collect::<Vec<_>>();
        let (packets, activity_spans) = if visible.len() <= request.max_items {
            (visible.into_iter().cloned().collect(), Vec::new())
        } else {
            let first = visible.first()?;
            let last = visible.last()?;
            (
                Vec::new(),
                vec![(
                    first.start_time_ns,
                    last.end_time_ns.max(first.start_time_ns),
                )],
            )
        };
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(
            ProtocolPacketLaneSnapshot {
                packets,
                activity_spans,
            },
        )))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        self.state
            .read()
            .unwrap()
            .packets
            .iter()
            .flat_map(|packet| [packet.start_time_ns, packet.end_time_ns])
            .filter(|boundary| boundary.abs_diff(timestamp_ns) <= max_distance_ns)
            .min_by_key(|boundary| boundary.abs_diff(timestamp_ns))
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        self.state
            .read()
            .unwrap()
            .packets
            .iter()
            .map(|packet| packet.end_time_ns)
            .max()
    }

    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        let state = self.state.read().unwrap();
        Some(CollectedLaneTableMetadata {
            generation: state.generation,
            total_rows: state.packets.len() as u64,
        })
    }

    fn table_snapshot(&self, max_rows: usize) -> Option<CollectedLaneTableSnapshot> {
        let state = self.state.read().unwrap();
        Some(CollectedLaneTableSnapshot {
            rows: state
                .packets
                .iter()
                .take(max_rows)
                .map(|packet| CollectedLaneTableRow {
                    start_time_ns: packet.start_time_ns,
                    end_time_ns: packet.end_time_ns,
                    value: 0,
                    payload: Some(WordPayload::Text(
                        protocol_packet_fallback_label(packet).into(),
                    )),
                })
                .collect(),
            complete: state.packets.len() <= max_rows,
            format_hint: Some("protocol-packet".to_owned()),
        })
    }

    fn storage_snapshot(&self) -> CollectedLaneStorageSnapshot {
        let state = self.state.read().unwrap();
        let payload_bytes = state
            .packets
            .iter()
            .map(|packet| packet.protocol_id.capacity() + protocol_value_bytes(&packet.value))
            .sum::<usize>();
        CollectedLaneStorageSnapshot {
            backing: CollectedLaneStorageBacking::Memory,
            retained_items: Some(state.packets.len() as u64),
            resident_bytes: Some(
                (state.packets.len() * size_of::<ProtocolPacket>() + payload_bytes) as u64,
            ),
            stored_bytes: None,
            index_items: None,
            index_bytes: None,
            live: self.live.load(Ordering::Acquire),
        }
    }
}

fn protocol_value_bytes(value: &ProtocolValue) -> usize {
    match value {
        ProtocolValue::String(value) => value.capacity(),
        ProtocolValue::Bytes(value) => value.len(),
        ProtocolValue::List(values) | ProtocolValue::Tuple(values) => {
            values.capacity() * size_of::<ProtocolValue>()
                + values.iter().map(protocol_value_bytes).sum::<usize>()
        }
        ProtocolValue::Mapping(values) => values
            .iter()
            .map(|(key, value)| key.capacity() + protocol_value_bytes(value))
            .sum(),
        ProtocolValue::Null
        | ProtocolValue::Bool(_)
        | ProtocolValue::Integer(_)
        | ProtocolValue::Float(_) => 0,
    }
}

struct ProtocolPacketLane {
    state: Arc<RwLock<RetainedState>>,
    live: Arc<AtomicBool>,
    buffer: VecDeque<ProtocolPacket>,
    retention: DerivedDataRetention,
    finished: bool,
}

impl ProtocolPacketLane {
    fn new(request: CollectedLaneRequest) -> Self {
        let state = Arc::new(RwLock::new(RetainedState {
            packets: Vec::new(),
            generation: 0,
        }));
        let live = Arc::new(AtomicBool::new(true));
        request.publish_query(Arc::new(ProtocolPacketLaneQuery {
            state: Arc::clone(&state),
            live: Arc::clone(&live),
        }));
        Self {
            state,
            live,
            buffer: VecDeque::new(),
            retention: request.retention(),
            finished: false,
        }
    }
}

impl CollectedLaneIngestor for ProtocolPacketLane {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<ProtocolPacket>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<ProtocolPacket>(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.finished = true,
            }
        } else {
            self.finished = true;
        }
        let count = batch.len();
        if count > 0 {
            let mut state = self.state.write().unwrap();
            state.packets.extend(batch);
            if let Some(target) = self.retention.trim_target(state.packets.len()) {
                let excess = state.packets.len() - target;
                state.packets.drain(..excess);
            }
            state.generation = state.generation.wrapping_add(1);
        }
        if self.finished {
            self.live.store(false, Ordering::Release);
        }
        Ok(count)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

struct ProtocolPacketPayloadAdapter;

impl PayloadAdapter for ProtocolPacketPayloadAdapter {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, PayloadIngestorConstructionError> {
        Ok(Box::new(ProtocolPacketLane::new(request)))
    }
}

/// Returns the generic retention adapter for [`ProtocolPacket`] payloads.
pub(crate) fn protocol_packet_payload_adapter() -> Arc<dyn PayloadAdapter> {
    Arc::new(ProtocolPacketPayloadAdapter)
}
