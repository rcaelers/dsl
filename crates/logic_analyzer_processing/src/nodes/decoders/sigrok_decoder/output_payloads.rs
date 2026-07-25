use std::any::Any;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use signal_processing::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneTableMetadata, CollectedLaneTableRow, CollectedLaneTableSnapshot,
    CollectedPayloadAdapter, DerivedDataRetention, InputPort, OpaqueCollectedLaneSnapshot,
    PortDirection, PortSchema, ProtocolPacket, WorkResult,
};

const DRAIN_BATCH_SIZE: usize = 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokAnnotation {
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub class: usize,
    pub rows: Arc<[usize]>,
    pub texts: Arc<[String]>,
}

impl SigrokAnnotation {
    pub fn display_text(&self) -> String {
        self.texts.first().cloned().unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokBinary {
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub class: usize,
    pub bytes: Arc<[u8]>,
}

impl SigrokBinary {
    pub fn display_text(&self) -> String {
        self.bytes
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigrokGeneratedLogic {
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub group: String,
    pub channel: String,
    pub samples: Arc<[u8]>,
    pub sample_count: usize,
}

impl SigrokGeneratedLogic {
    pub fn display_text(&self) -> String {
        format!(
            "{}.{} · {} samples",
            self.group, self.channel, self.sample_count
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SigrokMetadataValue {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SigrokMetadata {
    pub start_time_ns: u64,
    pub end_time_ns: u64,
    pub name: String,
    pub description: String,
    pub value: SigrokMetadataValue,
}

impl SigrokMetadata {
    pub fn display_text(&self) -> String {
        match self.value {
            SigrokMetadataValue::Unsigned(value) => format!("{}: {value}", self.name),
            SigrokMetadataValue::Signed(value) => format!("{}: {value}", self.name),
            SigrokMetadataValue::Float(value) => format!("{}: {value}", self.name),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SigrokLaneSnapshot<T> {
    entries: Vec<T>,
    activity_spans: Vec<(u64, u64)>,
}

impl<T> SigrokLaneSnapshot<T> {
    pub fn entries(&self) -> &[T] {
        &self.entries
    }

    /// Time buckets containing activity when an exact snapshot would exceed
    /// the viewer's bounded item budget.
    pub fn activity_spans(&self) -> &[(u64, u64)] {
        &self.activity_spans
    }
}

trait SigrokSpanPayload: Clone + Send + Sync + 'static {
    fn start_time_ns(&self) -> u64;
    fn end_time_ns(&self) -> u64;
    fn table_value(&self) -> u64;
    fn format_hint() -> &'static str;
}

macro_rules! impl_span_payload {
    ($type:ty, $table:expr, $format:literal) => {
        impl SigrokSpanPayload for $type {
            fn start_time_ns(&self) -> u64 {
                self.start_time_ns
            }

            fn end_time_ns(&self) -> u64 {
                self.end_time_ns
            }

            fn table_value(&self) -> u64 {
                $table(self)
            }

            fn format_hint() -> &'static str {
                $format
            }
        }
    };
}

impl_span_payload!(
    SigrokAnnotation,
    |value: &SigrokAnnotation| value.class as u64,
    "annotation-class"
);
impl_span_payload!(
    SigrokBinary,
    |value: &SigrokBinary| value.class as u64,
    "binary-class"
);
impl_span_payload!(
    SigrokGeneratedLogic,
    |value: &SigrokGeneratedLogic| value.sample_count as u64,
    "sample-count"
);
impl_span_payload!(
    SigrokMetadata,
    |value: &SigrokMetadata| metadata_table_value(value.value),
    "metadata-number"
);
impl_span_payload!(
    ProtocolPacket,
    |_value: &ProtocolPacket| 0,
    "protocol-packet"
);

struct RetainedState<T> {
    entries: Vec<T>,
    generation: u64,
}

struct SigrokLaneQuery<T> {
    state: Arc<RwLock<RetainedState<T>>>,
    live: Arc<AtomicBool>,
}

impl<T: SigrokSpanPayload> CollectedLaneQuery for SigrokLaneQuery<T> {
    fn into_any(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn snapshot(
        &self,
        request: CollectedLaneSnapshotRequest,
    ) -> Option<OpaqueCollectedLaneSnapshot> {
        let state = self.state.read().unwrap();
        let visible = state
            .entries
            .iter()
            .filter(|entry| {
                entry.end_time_ns() >= request.start_time_ns
                    && entry.start_time_ns() <= request.end_time_ns
            })
            .collect::<Vec<_>>();
        let (entries, activity_spans) = if visible.len() <= request.max_items {
            (visible.into_iter().cloned().collect(), Vec::new())
        } else {
            (
                Vec::new(),
                summarize_activity(
                    &visible,
                    request.start_time_ns,
                    request.end_time_ns,
                    request.max_items,
                ),
            )
        };
        Some(OpaqueCollectedLaneSnapshot::new(Arc::new(
            SigrokLaneSnapshot {
                entries,
                activity_spans,
            },
        )))
    }

    fn nearest_time_boundary(&self, timestamp_ns: u64, max_distance_ns: u64) -> Option<u64> {
        self.state
            .read()
            .unwrap()
            .entries
            .iter()
            .flat_map(|entry| [entry.start_time_ns(), entry.end_time_ns()])
            .filter_map(|boundary| {
                let distance = boundary.abs_diff(timestamp_ns);
                (distance <= max_distance_ns).then_some((distance, boundary))
            })
            .min()
            .map(|(_, boundary)| boundary)
    }

    fn timeline_extent_end_ns(&self) -> Option<u64> {
        self.state
            .read()
            .unwrap()
            .entries
            .iter()
            .map(SigrokSpanPayload::end_time_ns)
            .max()
    }

    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    fn table_metadata(&self) -> Option<CollectedLaneTableMetadata> {
        let state = self.state.read().unwrap();
        Some(CollectedLaneTableMetadata {
            generation: state.generation,
            total_rows: state.entries.len() as u64,
        })
    }

    fn table_snapshot(&self, max_rows: usize) -> Option<CollectedLaneTableSnapshot> {
        let state = self.state.read().unwrap();
        Some(CollectedLaneTableSnapshot {
            rows: state
                .entries
                .iter()
                .take(max_rows)
                .map(|entry| CollectedLaneTableRow {
                    start_time_ns: entry.start_time_ns(),
                    end_time_ns: entry.end_time_ns(),
                    value: entry.table_value(),
                })
                .collect(),
            complete: state.entries.len() <= max_rows,
            format_hint: Some(T::format_hint().to_owned()),
        })
    }
}

fn summarize_activity<T: SigrokSpanPayload>(
    entries: &[&T],
    start_time_ns: u64,
    end_time_ns: u64,
    max_items: usize,
) -> Vec<(u64, u64)> {
    let bucket_count = max_items.max(1);
    let duration = end_time_ns.saturating_sub(start_time_ns).saturating_add(1);
    let mut changes = vec![0_i64; bucket_count + 1];
    for entry in entries {
        let start = entry.start_time_ns().max(start_time_ns);
        let end = entry.end_time_ns().min(end_time_ns);
        let first = bucket_index(start, start_time_ns, duration, bucket_count);
        let last = bucket_index(end, start_time_ns, duration, bucket_count);
        changes[first] += 1;
        changes[last + 1] -= 1;
    }

    let mut spans = Vec::new();
    let mut active = 0_i64;
    let mut span_start = None;
    for (bucket, change) in changes.into_iter().take(bucket_count).enumerate() {
        active += change;
        match (span_start, active > 0) {
            (None, true) => span_start = Some(bucket),
            (Some(first), false) => {
                spans.push(bucket_span(
                    first,
                    bucket,
                    start_time_ns,
                    duration,
                    bucket_count,
                ));
                span_start = None;
            }
            _ => {}
        }
    }
    if let Some(first) = span_start {
        spans.push(bucket_span(
            first,
            bucket_count,
            start_time_ns,
            duration,
            bucket_count,
        ));
    }
    spans
}

fn bucket_index(time: u64, start: u64, duration: u64, bucket_count: usize) -> usize {
    ((u128::from(time.saturating_sub(start)) * bucket_count as u128 / u128::from(duration))
        as usize)
        .min(bucket_count - 1)
}

fn bucket_span(
    first: usize,
    end_exclusive: usize,
    start: u64,
    duration: u64,
    bucket_count: usize,
) -> (u64, u64) {
    let boundary = |bucket: usize| {
        start.saturating_add(
            (u128::from(duration) * bucket as u128 / bucket_count as u128).min(u128::from(u64::MAX))
                as u64,
        )
    };
    (boundary(first), boundary(end_exclusive).saturating_sub(1))
}

struct SigrokPayloadIngestor<T> {
    state: Arc<RwLock<RetainedState<T>>>,
    live: Arc<AtomicBool>,
    buffer: VecDeque<T>,
    retention: DerivedDataRetention,
    finished: bool,
}

impl<T: SigrokSpanPayload> SigrokPayloadIngestor<T> {
    fn new(request: CollectedLaneRequest) -> Self {
        let state = Arc::new(RwLock::new(RetainedState {
            entries: Vec::new(),
            generation: 0,
        }));
        let live = Arc::new(AtomicBool::new(true));
        request.publish_query(Arc::new(SigrokLaneQuery {
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

impl<T: SigrokSpanPayload> CollectedLaneIngestor for SigrokPayloadIngestor<T> {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<T>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::with_capacity(DRAIN_BATCH_SIZE);
        if let Some(mut receiver) = input.get::<T>(&mut self.buffer) {
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
            state.entries.extend(batch);
            if let Some(target) = self.retention.trim_target(state.entries.len()) {
                let excess = state.entries.len() - target;
                state.entries.drain(..excess);
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

struct SigrokPayloadAdapter<T>(std::marker::PhantomData<T>);

impl<T: SigrokSpanPayload> CollectedPayloadAdapter for SigrokPayloadAdapter<T> {
    fn create_ingestor(
        &self,
        request: CollectedLaneRequest,
    ) -> Result<Box<dyn CollectedLaneIngestor>, String> {
        Ok(Box::new(SigrokPayloadIngestor::<T>::new(request)))
    }
}

macro_rules! adapter_factory {
    ($name:ident, $type:ty) => {
        pub fn $name() -> Arc<dyn CollectedPayloadAdapter> {
            Arc::new(SigrokPayloadAdapter::<$type>(std::marker::PhantomData))
        }
    };
}

adapter_factory!(sigrok_annotation_payload_adapter, SigrokAnnotation);
adapter_factory!(sigrok_binary_payload_adapter, SigrokBinary);
adapter_factory!(sigrok_generated_logic_payload_adapter, SigrokGeneratedLogic);
adapter_factory!(sigrok_metadata_payload_adapter, SigrokMetadata);
adapter_factory!(sigrok_protocol_packet_payload_adapter, ProtocolPacket);

fn metadata_table_value(value: SigrokMetadataValue) -> u64 {
    match value {
        SigrokMetadataValue::Unsigned(value) => value,
        SigrokMetadataValue::Signed(value) => value as u64,
        SigrokMetadataValue::Float(value) => value.to_bits(),
    }
}

#[cfg(test)]
mod output_payload_tests {
    use crossbeam_channel::bounded;
    use signal_processing::{ChannelMessage, CollectedPayloadRegistry, DerivedLanes, Watchdog};

    use super::*;

    #[test]
    fn annotation_adapter_retains_spans_and_projects_snapshots_and_tables() {
        let lanes = DerivedLanes::new();
        let mut payloads = CollectedPayloadRegistry::new();
        payloads
            .register::<SigrokAnnotation>("org.logicconduit.sigrok.annotation/v1")
            .unwrap();
        let descriptor = payloads.descriptor::<SigrokAnnotation>().unwrap().clone();
        let mut ingestor = sigrok_annotation_payload_adapter()
            .create_ingestor(CollectedLaneRequest::new(
                "annotations",
                0,
                lanes.clone(),
                descriptor,
                DerivedDataRetention::MaxEntries(4),
            ))
            .unwrap();
        let watchdog = Watchdog::new();
        let (sender, receiver) = bounded(8);
        let input = InputPort::new_with_watchdog(receiver, &watchdog, "collector", "in0");
        sender
            .send(ChannelMessage::Batch(
                (0..6)
                    .map(|index| SigrokAnnotation {
                        start_time_ns: index * 10,
                        end_time_ns: index * 10 + 5,
                        class: index as usize,
                        rows: Arc::from([1]),
                        texts: Arc::from([format!("value {index}")]),
                    })
                    .collect(),
            ))
            .unwrap();
        assert_eq!(
            ingestor
                .drain(&input, DerivedDataRetention::Unlimited)
                .unwrap(),
            6
        );

        let lane = lanes.opaque_lanes().pop().unwrap();
        let query = lane.query::<SigrokLaneQuery<SigrokAnnotation>>().unwrap();
        let snapshot = query
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 30,
                end_time_ns: 60,
                max_items: 4,
            })
            .unwrap()
            .value::<SigrokLaneSnapshot<SigrokAnnotation>>()
            .unwrap();
        assert_eq!(
            snapshot
                .entries()
                .iter()
                .map(|entry| entry.class)
                .collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert!(snapshot.activity_spans().is_empty());

        let dense_snapshot = query
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 30,
                end_time_ns: 60,
                max_items: 2,
            })
            .unwrap()
            .value::<SigrokLaneSnapshot<SigrokAnnotation>>()
            .unwrap();
        assert!(dense_snapshot.entries().is_empty());
        assert_eq!(dense_snapshot.activity_spans(), [(30, 60)]);
        assert_eq!(query.nearest_time_boundary(46, 2), Some(45));
        assert_eq!(query.timeline_extent_end_ns(), Some(55));
        assert_eq!(query.table_metadata().unwrap().total_rows, 3);
        let table = query.table_snapshot(2).unwrap();
        assert_eq!(
            table.rows.iter().map(|row| row.value).collect::<Vec<_>>(),
            [3, 4]
        );
        assert!(!table.complete);
        assert_eq!(table.format_hint.as_deref(), Some("annotation-class"));

        drop(sender);
        ingestor
            .drain(&input, DerivedDataRetention::Unlimited)
            .unwrap();
        assert!(ingestor.is_finished());
        assert!(!query.is_live());
    }

    #[test]
    fn every_sigrok_payload_factory_owns_its_typed_input_schema() {
        assert_adapter_type::<SigrokAnnotation>(
            "org.logicconduit.sigrok.annotation/v1",
            sigrok_annotation_payload_adapter(),
        );
        assert_adapter_type::<SigrokBinary>(
            "org.logicconduit.sigrok.binary/v1",
            sigrok_binary_payload_adapter(),
        );
        assert_adapter_type::<SigrokGeneratedLogic>(
            "org.logicconduit.sigrok.generated-logic/v1",
            sigrok_generated_logic_payload_adapter(),
        );
        assert_adapter_type::<SigrokMetadata>(
            "org.logicconduit.sigrok.metadata/v1",
            sigrok_metadata_payload_adapter(),
        );
        assert_adapter_type::<ProtocolPacket>(
            "org.logicconduit.protocol-packet/v1",
            sigrok_protocol_packet_payload_adapter(),
        );
    }

    fn assert_adapter_type<T: Clone + Send + Sync + 'static>(
        stable_id: &str,
        adapter: Arc<dyn CollectedPayloadAdapter>,
    ) {
        let lanes = DerivedLanes::new();
        let mut payloads = CollectedPayloadRegistry::new();
        payloads.register::<T>(stable_id).unwrap();
        let descriptor = payloads.descriptor::<T>().unwrap().clone();
        let ingestor = adapter
            .create_ingestor(CollectedLaneRequest::new(
                stable_id,
                0,
                lanes,
                descriptor,
                DerivedDataRetention::Unlimited,
            ))
            .unwrap();
        assert_eq!(
            ingestor.input_schema(0).type_id,
            std::any::TypeId::of::<T>()
        );
    }
}
