use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, RwLock};

use crossbeam_channel::bounded;

use signal_capture::Sample;
use signal_runtime::{
    ChannelMessage, InputPort, OutputPort as OutPort, PortDirection, PortSchema, ProcessNode,
    Watchdog, WorkError, WorkResult,
};

use super::collector::DRAIN_BATCH_SIZE;
use super::digital::{DigitalLaneQuery, DigitalLaneStorage};
use super::number::{NumberLaneQuery, NumberLaneStorage};
use super::text::{TextLaneQuery, TextLaneStorage};
use super::timestamp_event::{TimestampEventLaneQuery, TimestampEventLaneStorage};
use super::word::{InMemoryWordLaneStorage, append_words_to_in_memory_storage};
use super::*;
use crate::derived_index::ChunkedMipmap;
use crate::derived_word_store::{IndexedAnnotationWriter, LiveStoreConfig, StoreStatus};
use crate::events::{Annotation, NumberSample, TextSample, TimestampEvent, Word};
use crate::payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, PayloadRegistry,
};

fn register_test_payload_adapters(registry: &mut PayloadRegistry) {
    registry
        .register::<Sample>("org.logicconduit.digital-sample/v1")
        .unwrap();
    registry
        .register_adapter::<Sample>(digital_payload_adapter())
        .unwrap();
    registry
        .register::<Word>("org.logicconduit.word/v1")
        .unwrap();
    registry
        .register_adapter::<Word>(word_payload_adapter())
        .unwrap();
    registry
        .register::<TimestampEvent>("org.logicconduit.trigger/v1")
        .unwrap();
    registry
        .register_adapter::<TimestampEvent>(timestamp_event_payload_adapter())
        .unwrap();
    registry
        .register::<NumberSample>("org.logicconduit.number-sample/v1")
        .unwrap();
    registry
        .register_adapter::<NumberSample>(number_payload_adapter())
        .unwrap();
    registry
        .register::<TextSample>("org.logicconduit.text-sample/v1")
        .unwrap();
    registry
        .register_adapter::<TextSample>(text_payload_adapter())
        .unwrap();
}

struct TestCollector {
    collector: DerivedDataCollector,
    lanes: DerivedLanes,
    retention: DerivedDataRetention,
    word_options: CollectedWordLaneOptions,
}

impl Deref for TestCollector {
    type Target = DerivedDataCollector;

    fn deref(&self) -> &Self::Target {
        &self.collector
    }
}

impl DerefMut for TestCollector {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.collector
    }
}

fn test_collector(lanes: DerivedLanes) -> TestCollector {
    TestCollector {
        collector: DerivedDataCollector::new(),
        lanes,
        retention: DerivedDataRetention::Unlimited,
        word_options: CollectedWordLaneOptions::default(),
    }
}

impl TestCollector {
    fn test_lane_request<T: Send + Sync + 'static>(
        &self,
        name: impl Into<String>,
    ) -> CollectedLaneRequest {
        let mut payloads = PayloadRegistry::new();
        register_test_payload_adapters(&mut payloads);
        CollectedLaneRequest::new(
            name,
            self.collector.num_inputs(),
            self.lanes.clone(),
            payloads.descriptor::<T>().unwrap().clone(),
            self.retention,
        )
    }

    fn with_indexed_words(mut self, indexed: bool) -> Self {
        self.word_options.set_indexed_for_test(indexed);
        self
    }

    fn with_word_store_config(mut self, store_config: LiveStoreConfig) -> Self {
        self.word_options.set_store_config_for_test(store_config);
        self
    }

    fn with_retention(mut self, retention: DerivedDataRetention) -> Self {
        self.retention = retention;
        self.collector = self.collector.with_retention(retention);
        self
    }

    fn with_ingestor(mut self, ingestor: Box<dyn CollectedLaneIngestor>) -> Self {
        self.collector = self.collector.with_ingestor(ingestor);
        self
    }

    fn with_number(mut self, name: impl Into<String>) -> Self {
        let request = self.test_lane_request::<NumberSample>(name);
        self.collector = self
            .collector
            .with_ingestor(number_payload_adapter().create_ingestor(request).unwrap());
        self
    }

    fn with_text(mut self, name: impl Into<String>) -> Self {
        let request = self.test_lane_request::<TextSample>(name);
        self.collector = self
            .collector
            .with_ingestor(text_payload_adapter().create_ingestor(request).unwrap());
        self
    }

    fn with_digital(mut self, name: impl Into<String>) -> Self {
        let request = self.test_lane_request::<Sample>(name);
        self.collector = self
            .collector
            .with_ingestor(digital_payload_adapter().create_ingestor(request).unwrap());
        self
    }

    fn with_trigger(mut self, name: impl Into<String>) -> Self {
        let request = self.test_lane_request::<TimestampEvent>(name);
        self.collector = self.collector.with_ingestor(
            timestamp_event_payload_adapter()
                .create_ingestor(request)
                .unwrap(),
        );
        self
    }

    fn with_words(mut self, name: impl Into<String>) -> Self {
        let request = self
            .test_lane_request::<Word>(name)
            .with_options(self.word_options.clone());
        self.collector = self
            .collector
            .with_ingestor(word_payload_adapter().create_ingestor(request).unwrap());
        self
    }
}

#[derive(Clone)]
struct PluginEvent(u64);

impl CollectedLaneQuery for std::sync::Mutex<Vec<u64>> {
    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

struct PluginEventIngestor {
    values: Arc<std::sync::Mutex<Vec<u64>>>,
    buffer: VecDeque<PluginEvent>,
    finished: bool,
}

impl CollectedLaneIngestor for PluginEventIngestor {
    fn input_schema(&self, index: usize) -> PortSchema {
        PortSchema::new::<PluginEvent>(format!("in{index}"), index, PortDirection::Input)
    }

    fn drain(&mut self, input: &InputPort, _retention: DerivedDataRetention) -> WorkResult<usize> {
        use crossbeam_channel::TryRecvError;

        let mut batch = Vec::new();
        if let Some(mut receiver) = input.get(&mut self.buffer) {
            match receiver.try_recv_many(&mut batch, DRAIN_BATCH_SIZE) {
                Ok(_) | Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.finished = true,
            }
        } else {
            self.finished = true;
        }
        let count = batch.len();
        self.values
            .lock()
            .unwrap()
            .extend(batch.into_iter().map(|event| event.0));
        Ok(count)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}
fn run_sink(sink: &mut DerivedDataCollector, inputs: Vec<InputPort>) {
    let outputs: Vec<OutPort> = vec![];
    loop {
        match sink.work(&inputs, &outputs) {
            Ok(_) => {}
            Err(WorkError::Shutdown) => break,
            Err(e) => panic!("unexpected error: {e}"),
        }
    }
}

fn lane(lanes: &DerivedLanes, name: &str) -> OpaqueCollectedLane {
    lanes
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.name() == name)
        .unwrap_or_else(|| panic!("missing collected lane {name}"))
}

fn lane_snapshot<T: Send + Sync + 'static>(
    lane: &OpaqueCollectedLane,
    start_time_ns: u64,
    end_time_ns: u64,
    max_items: usize,
) -> Arc<T> {
    lane.snapshot(CollectedLaneSnapshotRequest {
        start_time_ns,
        end_time_ns,
        max_items,
    })
    .and_then(|snapshot| snapshot.value::<T>())
    .unwrap_or_else(|| panic!("{} did not publish the expected snapshot", lane.name()))
}

fn append_test_words(
    storage: &mut InMemoryWordLaneStorage,
    words: impl IntoIterator<Item = (u64, u64, u64)>,
) {
    let words = words
        .into_iter()
        .map(|(timestamp_ns, duration_ns, value)| Word::spanning(value, timestamp_ns, duration_ns))
        .collect::<Vec<_>>();
    append_words_to_in_memory_storage(storage, &words, DerivedDataRetention::Unlimited);
}

#[test]
fn adapter_lane_collects_and_publishes_an_opaque_query() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    payloads
        .register::<PluginEvent>("org.example.plugin-event/v1")
        .unwrap();
    let values = Arc::new(std::sync::Mutex::new(Vec::new()));
    lanes.publish_opaque_lane(
        "plugin.events",
        payloads.descriptor::<PluginEvent>().unwrap().clone(),
        Arc::clone(&values),
    );
    let mut collector =
        test_collector(lanes.clone()).with_ingestor(Box::new(PluginEventIngestor {
            values: Arc::clone(&values),
            buffer: VecDeque::new(),
            finished: false,
        }));

    let watchdog = Watchdog::new();
    let (sender, receiver) = bounded::<ChannelMessage<PluginEvent>>(4);
    sender
        .send(ChannelMessage::Batch(vec![PluginEvent(2), PluginEvent(5)]))
        .unwrap();
    drop(sender);
    run_sink(
        &mut collector,
        vec![InputPort::new_with_watchdog(
            receiver,
            &watchdog,
            "collector",
            "in0",
        )],
    );

    assert_eq!(*values.lock().unwrap(), vec![2, 5]);
    let opaque = lanes.opaque_lanes();
    assert_eq!(opaque[0].name(), "plugin.events");
    assert_eq!(
        opaque[0].payload().stable_id(),
        "org.example.plugin-event/v1"
    );
    assert_eq!(
        *opaque[0]
            .query::<std::sync::Mutex<Vec<u64>>>()
            .unwrap()
            .lock()
            .unwrap(),
        vec![2, 5]
    );
}

#[test]
fn built_in_payloads_register_through_the_adapter_registry() {
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);

    for type_id in [
        std::any::TypeId::of::<Sample>(),
        std::any::TypeId::of::<Word>(),
        std::any::TypeId::of::<TimestampEvent>(),
        std::any::TypeId::of::<NumberSample>(),
        std::any::TypeId::of::<TextSample>(),
    ] {
        assert!(payloads.adapter_by_type_id(type_id).is_some());
    }
}

#[test]
fn adapter_owned_lane_publishes_its_payload_identity() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let descriptor = payloads.descriptor::<Word>().unwrap().clone();
    let ingestor = payloads
        .adapter_by_type_id(std::any::TypeId::of::<Word>())
        .unwrap()
        .create_ingestor(
            CollectedLaneRequest::new(
                "words",
                0,
                lanes.clone(),
                descriptor,
                DerivedDataRetention::Unlimited,
            )
            .with_options(CollectedWordLaneOptions::in_memory_for_test()),
        )
        .unwrap();

    let _collector = DerivedDataCollector::new().with_ingestor(ingestor);
    assert_eq!(
        lanes.opaque_lanes()[0].payload().stable_id(),
        "org.logicconduit.word/v1"
    );
    assert_eq!(
        lanes.opaque_lanes()[0].storage_snapshot(),
        CollectedLaneStorageSnapshot {
            backing: CollectedLaneStorageBacking::Memory,
            retained_items: Some(0),
            resident_bytes: Some(0),
            stored_bytes: None,
            index_items: Some(0),
            index_bytes: Some(0),
            live: false,
        }
    );
    let snapshot = lanes.opaque_lanes()[0]
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 1,
            max_items: 8,
        })
        .unwrap();
    assert!(matches!(
        snapshot.value::<WordLaneSnapshot>().as_deref(),
        Some(WordLaneSnapshot::Exact { annotations, .. }) if annotations.is_empty()
    ));
}

#[test]
fn arbitrary_width_words_are_retained_without_numeric_truncation() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let descriptor = payloads.descriptor::<Word>().unwrap().clone();
    let ingestor = payloads
        .adapter_by_type_id(std::any::TypeId::of::<Word>())
        .unwrap()
        .create_ingestor(
            CollectedLaneRequest::new(
                "wide.words",
                0,
                lanes.clone(),
                descriptor,
                DerivedDataRetention::Unlimited,
            )
            .with_options(CollectedWordLaneOptions::in_memory_for_test()),
        )
        .unwrap();
    let mut collector = test_collector(lanes.clone()).with_ingestor(ingestor);
    let watchdog = Watchdog::new();
    let (sender, receiver) = bounded(4);
    let bytes: Arc<[u8]> = (0..32).collect::<Vec<_>>().into();
    sender
        .send(ChannelMessage::Sample(Word::bytes(
            Arc::clone(&bytes),
            10,
            20,
        )))
        .unwrap();
    drop(sender);
    run_sink(
        &mut collector,
        vec![InputPort::new_with_watchdog(
            receiver,
            &watchdog,
            "collector",
            "in0",
        )],
    );

    let snapshot = lanes.opaque_lanes()[0]
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 100,
            max_items: 8,
        })
        .unwrap()
        .value::<WordLaneSnapshot>()
        .unwrap();
    let WordLaneSnapshot::Exact { annotations, .. } = snapshot.as_ref() else {
        panic!("expected exact rich-word snapshot");
    };
    assert_eq!(annotations.len(), 1);
    assert_eq!(
        annotations[0].payload,
        Some(crate::WordPayload::Bytes(bytes))
    );
    let storage = lanes.opaque_lanes()[0].storage_snapshot();
    assert_eq!(storage.backing, CollectedLaneStorageBacking::Memory);
    assert_eq!(storage.retained_items, Some(1));
    assert!(storage.resident_bytes.unwrap() >= 32);
    assert_eq!(storage.index_items, Some(1));
}

#[test]
fn standalone_word_ingestor_publishes_only_its_query() {
    let lanes = DerivedLanes::new();
    let options = CollectedWordLaneOptions::in_memory_for_test();

    let _ingestor = built_in_word_lane_ingestor(
        "words",
        lanes.clone(),
        DerivedDataRetention::Unlimited,
        options,
        crate::DecodedBlockCacheHandle::default(),
    );

    assert!(
        lanes.opaque_lanes()[0]
            .query::<CollectedWordLaneQuery>()
            .is_some()
    );
}

#[test]
fn digital_adapter_publishes_an_opaque_snapshot_query() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let descriptor = payloads.descriptor::<Sample>().unwrap().clone();
    let ingestor = payloads
        .adapter_by_type_id(std::any::TypeId::of::<Sample>())
        .unwrap()
        .create_ingestor(CollectedLaneRequest::new(
            "signal",
            0,
            lanes.clone(),
            descriptor,
            DerivedDataRetention::Unlimited,
        ))
        .unwrap();

    let _collector = DerivedDataCollector::new().with_ingestor(ingestor);
    let snapshot = lanes.opaque_lanes()[0]
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 1,
            max_items: 8,
        })
        .unwrap();
    assert!(matches!(
        snapshot.value::<DigitalLaneSnapshot>().as_deref(),
        Some(DigitalLaneSnapshot::Exact { samples, initial: false }) if samples.is_empty()
    ));
}

#[test]
fn trigger_adapter_publishes_an_opaque_snapshot_query() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let descriptor = payloads.descriptor::<TimestampEvent>().unwrap().clone();
    let ingestor = payloads
        .adapter_by_type_id(std::any::TypeId::of::<TimestampEvent>())
        .unwrap()
        .create_ingestor(CollectedLaneRequest::new(
            "trigger",
            0,
            lanes.clone(),
            descriptor,
            DerivedDataRetention::Unlimited,
        ))
        .unwrap();

    let _collector = DerivedDataCollector::new().with_ingestor(ingestor);
    let snapshot = lanes.opaque_lanes()[0]
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 1,
            max_items: 8,
        })
        .unwrap();
    assert!(matches!(
        snapshot.value::<TimestampEventLaneSnapshot>().as_deref(),
        Some(TimestampEventLaneSnapshot::Exact(markers)) if markers.is_empty()
    ));
}

#[test]
fn level_adapters_publish_typed_snapshots_after_collection() {
    let lanes = DerivedLanes::new();
    let mut payloads = crate::PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let number = payloads
        .adapter_by_type_id(std::any::TypeId::of::<NumberSample>())
        .unwrap()
        .create_ingestor(CollectedLaneRequest::new(
            "number",
            0,
            lanes.clone(),
            payloads.descriptor::<NumberSample>().unwrap().clone(),
            DerivedDataRetention::Unlimited,
        ))
        .unwrap();
    let text = payloads
        .adapter_by_type_id(std::any::TypeId::of::<TextSample>())
        .unwrap()
        .create_ingestor(CollectedLaneRequest::new(
            "text",
            1,
            lanes.clone(),
            payloads.descriptor::<TextSample>().unwrap().clone(),
            DerivedDataRetention::Unlimited,
        ))
        .unwrap();
    let mut collector = DerivedDataCollector::new()
        .with_ingestor(number)
        .with_ingestor(text);
    let watchdog = Watchdog::new();
    let (number_sender, number_receiver) = bounded::<ChannelMessage<NumberSample>>(4);
    number_sender
        .send(ChannelMessage::Sample(NumberSample::new(-7, 100)))
        .unwrap();
    drop(number_sender);
    let (text_sender, text_receiver) = bounded::<ChannelMessage<TextSample>>(4);
    text_sender
        .send(ChannelMessage::Sample(TextSample::new("ready", 100)))
        .unwrap();
    drop(text_sender);
    run_sink(
        &mut collector,
        vec![
            InputPort::new_with_watchdog(number_receiver, &watchdog, "collector", "in0"),
            InputPort::new_with_watchdog(text_receiver, &watchdog, "collector", "in1"),
        ],
    );

    let opaque = lanes.opaque_lanes();
    assert!(matches!(
        opaque[0]
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 0,
                end_time_ns: 200,
                max_items: 8,
            })
            .and_then(|snapshot| snapshot.value::<NumberLaneSnapshot>())
            .as_deref(),
        Some(NumberLaneSnapshot::Exact(samples)) if samples == &[NumberSample::new(-7, 100)]
    ));
    assert!(matches!(
        opaque[1]
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 0,
                end_time_ns: 200,
                max_items: 8,
            })
            .and_then(|snapshot| snapshot.value::<TextLaneSnapshot>())
            .as_deref(),
        Some(TextLaneSnapshot::Exact(samples)) if samples == &[TextSample::new("ready", 100)]
    ));
}

#[test]
fn built_in_scalar_and_event_adapters_reopen_persistent_indexed_lanes() {
    let configs = (1_u8..=4)
        .map(|key| LiveStoreConfig {
            persistence: Some(crate::derived_word_store::PersistentStoreConfig::new(
                [key; 32],
            )),
            ..LiveStoreConfig::default()
        })
        .collect::<Vec<_>>();
    let mut payloads = PayloadRegistry::new();
    register_test_payload_adapters(&mut payloads);
    let lanes = DerivedLanes::new();
    let digital = digital_payload_adapter()
        .create_ingestor(
            CollectedLaneRequest::new(
                "digital",
                0,
                lanes.clone(),
                payloads.descriptor::<Sample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[0].clone()),
        )
        .unwrap();
    let number = number_payload_adapter()
        .create_ingestor(
            CollectedLaneRequest::new(
                "number",
                1,
                lanes.clone(),
                payloads.descriptor::<NumberSample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[1].clone()),
        )
        .unwrap();
    let text = text_payload_adapter()
        .create_ingestor(
            CollectedLaneRequest::new(
                "text",
                2,
                lanes.clone(),
                payloads.descriptor::<TextSample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[2].clone()),
        )
        .unwrap();
    let trigger = timestamp_event_payload_adapter()
        .create_ingestor(
            CollectedLaneRequest::new(
                "trigger",
                3,
                lanes.clone(),
                payloads.descriptor::<TimestampEvent>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[3].clone()),
        )
        .unwrap();
    let mut collector = DerivedDataCollector::new()
        .with_ingestor(digital)
        .with_ingestor(number)
        .with_ingestor(text)
        .with_ingestor(trigger);
    let watchdog = Watchdog::new();
    let (digital_sender, digital_receiver) = bounded(4);
    digital_sender
        .send(ChannelMessage::Sample(Sample::new(true, 10)))
        .unwrap();
    drop(digital_sender);
    let (number_sender, number_receiver) = bounded(4);
    number_sender
        .send(ChannelMessage::Sample(NumberSample::new(-7, 20)))
        .unwrap();
    drop(number_sender);
    let (text_sender, text_receiver) = bounded(4);
    text_sender
        .send(ChannelMessage::Sample(TextSample::new("cached", 30)))
        .unwrap();
    drop(text_sender);
    let (trigger_sender, trigger_receiver) = bounded(4);
    trigger_sender
        .send(ChannelMessage::Sample(TimestampEvent { timestamp_ns: 40 }))
        .unwrap();
    drop(trigger_sender);
    run_sink(
        &mut collector,
        vec![
            InputPort::new_with_watchdog(digital_receiver, &watchdog, "collector", "in0"),
            InputPort::new_with_watchdog(number_receiver, &watchdog, "collector", "in1"),
            InputPort::new_with_watchdog(text_receiver, &watchdog, "collector", "in2"),
            InputPort::new_with_watchdog(trigger_receiver, &watchdog, "collector", "in3"),
        ],
    );

    let reopened_lanes = DerivedLanes::new();
    let reopened = [
        digital_payload_adapter().create_ingestor(
            CollectedLaneRequest::new(
                "digital",
                0,
                reopened_lanes.clone(),
                payloads.descriptor::<Sample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[0].clone()),
        ),
        number_payload_adapter().create_ingestor(
            CollectedLaneRequest::new(
                "number",
                1,
                reopened_lanes.clone(),
                payloads.descriptor::<NumberSample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[1].clone()),
        ),
        text_payload_adapter().create_ingestor(
            CollectedLaneRequest::new(
                "text",
                2,
                reopened_lanes.clone(),
                payloads.descriptor::<TextSample>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[2].clone()),
        ),
        timestamp_event_payload_adapter().create_ingestor(
            CollectedLaneRequest::new(
                "trigger",
                3,
                reopened_lanes.clone(),
                payloads.descriptor::<TimestampEvent>().unwrap().clone(),
                DerivedDataRetention::Unlimited,
            )
            .with_indexed_store(configs[3].clone()),
        ),
    ];
    assert!(reopened.iter().all(Result::is_ok));

    assert!(matches!(
        lane_snapshot::<DigitalLaneSnapshot>(&lane(&reopened_lanes, "digital"), 0, 100, 8)
            .as_ref(),
        DigitalLaneSnapshot::Exact { samples, initial: false }
            if samples == &[Sample::new(true, 10)]
    ));
    assert!(matches!(
        lane_snapshot::<NumberLaneSnapshot>(&lane(&reopened_lanes, "number"), 0, 100, 8).as_ref(),
        NumberLaneSnapshot::Exact(samples) if samples == &[NumberSample::new(-7, 20)]
    ));
    assert!(matches!(
        lane_snapshot::<TextLaneSnapshot>(&lane(&reopened_lanes, "text"), 0, 100, 8).as_ref(),
        TextLaneSnapshot::Exact(samples) if samples == &[TextSample::new("cached", 30)]
    ));
    assert!(matches!(
        lane_snapshot::<TimestampEventLaneSnapshot>(&lane(&reopened_lanes, "trigger"), 0, 100, 8)
            .as_ref(),
        TimestampEventLaneSnapshot::Exact(markers) if markers == &[40]
    ));
}

#[test]
fn word_query_returns_only_a_bounded_visible_snapshot() {
    let query = CollectedWordLaneQuery::in_memory_for_test(InMemoryWordLaneStorage {
        annotations: vec![
            Annotation {
                start_ns: 10,
                end_ns: 20,
                value: 1,
                payload: None,
            },
            Annotation {
                start_ns: 30,
                end_ns: 40,
                value: 2,
                payload: None,
            },
            Annotation {
                start_ns: 50,
                end_ns: 60,
                value: 3,
                payload: None,
            },
        ],
        summary: ChunkedMipmap::new(),
        generation: 60,
    });

    assert!(matches!(
        query.snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 25,
            end_time_ns: 45,
            max_items: 2,
        }),
        WordLaneSnapshot::Exact { annotations, .. }
            if annotations.iter().map(|annotation| annotation.value).eq([2])
    ));
    assert!(matches!(
        query.snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 100,
            max_items: 2,
        }),
        WordLaneSnapshot::Activity
    ));
    assert_eq!(query.nearest_time_boundary(19, 3), Some(20));
    assert_eq!(query.nearest_time_boundary(25, 3), None);
    assert_eq!(query.timeline_extent_end_ns(), Some(60));
    assert_eq!(
        query.table_metadata(),
        Some(CollectedLaneTableMetadata {
            generation: 60,
            total_rows: 3,
        })
    );
    assert_eq!(
        query.table_snapshot(2),
        Some(CollectedLaneTableSnapshot {
            rows: vec![
                CollectedLaneTableRow {
                    start_time_ns: 10,
                    end_time_ns: 20,
                    value: 1,
                    payload: None,
                },
                CollectedLaneTableRow {
                    start_time_ns: 30,
                    end_time_ns: 40,
                    value: 2,
                    payload: None,
                },
            ],
            complete: false,
            format_hint: None,
        })
    );
}

#[test]
fn lanes_collect_signals_words_and_triggers() {
    let store = DerivedLanes::new();
    let mut sink = test_collector(store.clone())
        .with_digital("latch.q")
        .with_words("decoder.words")
        .with_trigger("start.match");

    let wd = Watchdog::new();
    let (sig_tx, sig_rx) = bounded::<ChannelMessage<Sample>>(16);
    sig_tx
        .send(ChannelMessage::Sample(Sample::new(true, 100)))
        .unwrap();
    sig_tx
        .send(ChannelMessage::Sample(Sample::new(false, 300)))
        .unwrap();
    drop(sig_tx);

    let (word_tx, word_rx) = bounded::<ChannelMessage<Word>>(16);
    for (value, ts) in [(0xAB_u64, 1_000_u64), (0xCD, 1_500)] {
        word_tx
            .send(ChannelMessage::Sample(Word::new(value, ts)))
            .unwrap();
    }
    drop(word_tx);

    let (trig_tx, trig_rx) = bounded::<ChannelMessage<TimestampEvent>>(16);
    trig_tx
        .send(ChannelMessage::Sample(TimestampEvent { timestamp_ns: 42 }))
        .unwrap();
    drop(trig_tx);

    let inputs = vec![
        InputPort::new_with_watchdog(sig_rx, &wd, "viewer", "in0"),
        InputPort::new_with_watchdog(word_rx, &wd, "viewer", "in1"),
        InputPort::new_with_watchdog(trig_rx, &wd, "viewer", "in2"),
    ];
    run_sink(&mut sink, inputs);

    let lanes = store.opaque_lanes();
    assert_eq!(lanes.len(), 3);
    assert_eq!(lanes[0].name(), "latch.q");
    assert!(matches!(
        lane_snapshot::<DigitalLaneSnapshot>(&lanes[0], 0, 2_000, 10).as_ref(),
        DigitalLaneSnapshot::Exact { samples, initial: false }
            if samples == &[Sample::new(true, 100), Sample::new(false, 300)]
    ));
    let expected = [
        Annotation {
            start_ns: 1_000,
            end_ns: 1_500,
            value: 0xAB,
            payload: None,
        },
        Annotation {
            start_ns: 1_500,
            end_ns: 1_500,
            value: 0xCD,
            payload: None,
        },
    ];
    let indexed = lanes[1]
        .query::<CollectedWordLaneQuery>()
        .and_then(|query| query.indexed_lane())
        .expect("expected indexed word lane");
    assert_eq!(indexed.status(), StoreStatus::Finished);
    assert_eq!(indexed.metadata().total_word_count, 2);
    assert_eq!(
        indexed
            .query()
            .exact_window(0, 2_000, 10)
            .unwrap()
            .annotations,
        expected
    );
    assert!(matches!(
        lane_snapshot::<TimestampEventLaneSnapshot>(&lanes[2], 0, 2_000, 10).as_ref(),
        TimestampEventLaneSnapshot::Exact(markers) if markers == &[42]
    ));
}

#[test]
fn lanes_collect_number_and_text_levels() {
    let store = DerivedLanes::new();
    let mut sink = test_collector(store.clone())
        .with_number("counter.count")
        .with_text("formatter.text");

    let wd = Watchdog::new();
    let (number_tx, number_rx) = bounded::<ChannelMessage<NumberSample>>(16);
    number_tx
        .send(ChannelMessage::Sample(NumberSample::new(-2, 0)))
        .unwrap();
    number_tx
        .send(ChannelMessage::Sample(NumberSample::new(3, 500)))
        .unwrap();
    drop(number_tx);

    let (text_tx, text_rx) = bounded::<ChannelMessage<TextSample>>(16);
    text_tx
        .send(ChannelMessage::Sample(TextSample::new("Window 03", 500)))
        .unwrap();
    drop(text_tx);

    run_sink(
        &mut sink,
        vec![
            InputPort::new_with_watchdog(number_rx, &wd, "viewer", "in0"),
            InputPort::new_with_watchdog(text_rx, &wd, "viewer", "in1"),
        ],
    );

    assert!(matches!(
        lane_snapshot::<NumberLaneSnapshot>(&lane(&store, "counter.count"), 0, 1_000, 10)
            .as_ref(),
        NumberLaneSnapshot::Exact(samples)
            if samples == &[NumberSample::new(-2, 0), NumberSample::new(3, 500)]
    ));
    assert!(matches!(
        lane_snapshot::<TextLaneSnapshot>(&lane(&store, "formatter.text"), 0, 1_000, 10)
            .as_ref(),
        TextLaneSnapshot::Exact(samples)
            if samples == &[TextSample::new("Window 03", 500)]
    ));
}

#[test]
fn work_drains_at_most_one_batch_per_call() {
    // A single `work()` call must not race a fast producer to keep the
    // channel empty — that's what lets the channel's own bound and
    // `Block` overflow policy apply real backpressure instead of never
    // engaging (§`DRAIN_BATCH_SIZE`).
    let store = DerivedLanes::new();
    let mut sink = test_collector(store.clone()).with_digital("sig");

    let total = DRAIN_BATCH_SIZE + 5;
    let wd = Watchdog::new();
    let (tx, rx) = bounded::<ChannelMessage<Sample>>(total + 1);
    for i in 0..total as u64 {
        tx.send(ChannelMessage::Sample(Sample::new(i % 2 == 0, i)))
            .unwrap();
    }
    drop(tx);
    let inputs = vec![InputPort::new_with_watchdog(rx, &wd, "viewer", "in0")];

    let progress = sink.work(&inputs, &[]).unwrap();
    assert_eq!(progress, DRAIN_BATCH_SIZE, "one call drains one batch");
    assert_eq!(
        lane(&store, "sig").storage_snapshot().retained_items,
        Some(DRAIN_BATCH_SIZE as u64)
    );

    // The remainder (plus the shutdown sentinel) arrives over the
    // following calls.
    run_sink(&mut sink, inputs);
    assert_eq!(
        lane(&store, "sig").storage_snapshot().retained_items,
        Some(total as u64)
    );
}

#[test]
fn instantaneous_annotation_leaves_long_inter_word_gaps_empty() {
    let mut storage = InMemoryWordLaneStorage {
        annotations: Vec::new(),
        summary: ChunkedMipmap::new(),
        generation: 0,
    };
    append_test_words(
        &mut storage,
        [
            (1_000, 0, 1),
            (1_100, 0, 2),
            (1_100 + crate::events::MAX_ANNOTATION_NS * 10, 0, 3),
        ],
    );
    let annotations = &storage.annotations;
    assert_eq!(annotations[0].end_ns, 1_100);
    assert_eq!(annotations[1].end_ns, 1_200);
    assert!(annotations[1].end_ns < annotations[2].start_ns);
}

#[test]
fn instantaneous_annotations_follow_a_slow_burst_cadence() {
    const WORD_PERIOD_NS: u64 = 24_000_000;
    let mut storage = InMemoryWordLaneStorage {
        annotations: Vec::new(),
        summary: ChunkedMipmap::new(),
        generation: 0,
    };
    append_test_words(
        &mut storage,
        [
            (1_000_000_000, 0, 1),
            (1_000_000_000 + WORD_PERIOD_NS, 0, 2),
            (1_000_000_000 + WORD_PERIOD_NS * 2, 0, 3),
            (6_000_000_000, 0, 4),
        ],
    );

    let annotations = &storage.annotations;
    assert_eq!(annotations[0].end_ns, annotations[1].start_ns);
    assert_eq!(annotations[1].end_ns, annotations[2].start_ns);
    assert_eq!(
        annotations[2].end_ns,
        annotations[2].start_ns + WORD_PERIOD_NS
    );
    assert!(annotations[2].end_ns < annotations[3].start_ns);
}

#[test]
fn digital_query_returns_bounded_exact_or_activity_snapshots() {
    let mut storage = DigitalLaneStorage::default();
    for sample in [
        Sample::new(true, 100),
        Sample::new(false, 200),
        Sample::new(true, 300),
    ] {
        storage.summary.push(&sample);
        storage.samples.push(sample);
    }
    let query = DigitalLaneQuery {
        storage: Arc::new(RwLock::new(storage)),
        indexed: None,
    };

    let exact = query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 150,
            end_time_ns: 350,
            max_items: 3,
        })
        .and_then(|snapshot| snapshot.value::<DigitalLaneSnapshot>())
        .expect("digital exact snapshot");
    assert!(matches!(
        exact.as_ref(),
        DigitalLaneSnapshot::Exact { samples, initial: true }
            if samples == &[Sample::new(false, 200), Sample::new(true, 300)]
    ));

    let activity = query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 400,
            max_items: 1,
        })
        .and_then(|snapshot| snapshot.value::<DigitalLaneSnapshot>())
        .expect("digital activity snapshot");
    assert!(matches!(
        activity.as_ref(),
        DigitalLaneSnapshot::Activity { records, initial: false } if !records.is_empty()
    ));
    assert_eq!(query.timeline_extent_end_ns(), Some(300));
    assert_eq!(query.nearest_time_boundary(190, 20), Some(200));
}

#[test]
fn trigger_query_returns_bounded_exact_or_activity_snapshots() {
    let mut storage = TimestampEventLaneStorage::default();
    for timestamp_ns in [100, 200, 300] {
        storage.summary.push(&timestamp_ns);
        storage.timestamps.push(timestamp_ns);
    }
    let query = TimestampEventLaneQuery {
        storage: Arc::new(RwLock::new(storage)),
        indexed: None,
    };

    let exact = query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 150,
            end_time_ns: 350,
            max_items: 3,
        })
        .and_then(|snapshot| snapshot.value::<TimestampEventLaneSnapshot>())
        .expect("trigger exact snapshot");
    assert!(matches!(
        exact.as_ref(),
        TimestampEventLaneSnapshot::Exact(markers) if markers == &[200, 300]
    ));

    let activity = query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 400,
            max_items: 1,
        })
        .and_then(|snapshot| snapshot.value::<TimestampEventLaneSnapshot>())
        .expect("trigger activity snapshot");
    assert!(matches!(
        activity.as_ref(),
        TimestampEventLaneSnapshot::Activity(records) if !records.is_empty()
    ));
    assert_eq!(query.timeline_extent_end_ns(), Some(300));
    assert_eq!(query.nearest_time_boundary(190, 20), Some(200));
}

#[test]
fn number_and_text_queries_preserve_typed_values_and_bound_dense_windows() {
    let number_storage = Arc::new(RwLock::new(NumberLaneStorage::default()));
    {
        let mut storage = number_storage.write().unwrap();
        for sample in [NumberSample::new(-2, 100), NumberSample::new(3, 200)] {
            storage.summary.push(&sample);
            storage.values.push(sample);
        }
    }
    let number_query = NumberLaneQuery {
        storage: Arc::clone(&number_storage),
        indexed: None,
    };
    let number_exact = number_query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 150,
            end_time_ns: 250,
            max_items: 2,
        })
        .and_then(|snapshot| snapshot.value::<NumberLaneSnapshot>())
        .expect("numeric exact snapshot");
    assert!(matches!(
        number_exact.as_ref(),
        NumberLaneSnapshot::Exact(samples)
            if samples == &[NumberSample::new(-2, 100), NumberSample::new(3, 200)]
    ));
    let number_activity = number_query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 300,
            max_items: 1,
        })
        .and_then(|snapshot| snapshot.value::<NumberLaneSnapshot>())
        .expect("numeric activity snapshot");
    assert!(matches!(
        number_activity.as_ref(),
        NumberLaneSnapshot::Activity(records) if !records.is_empty()
    ));

    let text_storage = Arc::new(RwLock::new(TextLaneStorage::default()));
    {
        let mut storage = text_storage.write().unwrap();
        for sample in [TextSample::new("one", 100), TextSample::new("two", 200)] {
            storage.summary.push(&sample);
            storage.values.push(sample);
        }
    }
    let text_query = TextLaneQuery {
        storage: Arc::clone(&text_storage),
        indexed: None,
    };
    let text_exact = text_query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 150,
            end_time_ns: 250,
            max_items: 2,
        })
        .and_then(|snapshot| snapshot.value::<TextLaneSnapshot>())
        .expect("text exact snapshot");
    assert!(matches!(
        text_exact.as_ref(),
        TextLaneSnapshot::Exact(samples)
            if samples == &[TextSample::new("one", 100), TextSample::new("two", 200)]
    ));
    let text_activity = text_query
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: 300,
            max_items: 1,
        })
        .and_then(|snapshot| snapshot.value::<TextLaneSnapshot>())
        .expect("text activity snapshot");
    assert!(matches!(
        text_activity.as_ref(),
        TextLaneSnapshot::Activity(records) if !records.is_empty()
    ));
    assert_eq!(text_query.timeline_extent_end_ns(), Some(200));
    assert_eq!(text_query.nearest_time_boundary(190, 20), Some(200));
}

/// A word carrying a real duration is stored closed at its
/// true end immediately — never patched to the next word's start, never
/// left open for the renderer to estimate.
#[test]
fn word_with_duration_is_closed_at_its_true_end() {
    let mut storage = InMemoryWordLaneStorage {
        annotations: Vec::new(),
        summary: ChunkedMipmap::new(),
        generation: 0,
    };

    // A word spanning 2_300ns, followed much later by another; the first's
    // end must stay its own, not stretch to the second's start.
    append_test_words(&mut storage, [(1_000, 2_300, 0x600081)]);
    assert_eq!(
        storage.annotations.as_slice(),
        &[Annotation {
            start_ns: 1_000,
            end_ns: 3_300,
            value: 0x600081,
            payload: None,
        }]
    );
    // Closed immediately → in the summary at once, no one-entry lag.
    assert_eq!(storage.summary.len(), 1);

    append_test_words(&mut storage, [(500_000, 2_300, 0x600000)]);
    let annotations = &storage.annotations;
    assert_eq!(annotations[0].end_ns, 3_300, "true end must not be patched");
    assert_eq!(annotations[1].end_ns, 502_300);
}

#[test]
fn summary_lags_the_most_recent_open_annotation_by_one() {
    // The mipmap can't retroactively patch an entry once it's pushed,
    // so the most recent (still "open", not yet end-patched) annotation
    // only joins the summary once the *next* word closes it.
    let mut storage = InMemoryWordLaneStorage {
        annotations: Vec::new(),
        summary: ChunkedMipmap::new(),
        generation: 0,
    };

    append_test_words(&mut storage, [(1_000, 0, 0xAB)]);
    assert_eq!(
        storage.summary.len(),
        0,
        "the only word so far is still open"
    );

    append_test_words(&mut storage, [(1_500, 0, 0xCD)]);
    assert_eq!(storage.summary.len(), 1, "the first word is now closed");
    let window = storage.summary.sampled_window(0, 1_500, 10);
    assert_eq!(window[0].start_ns, 1_000);
    assert_eq!(window[0].end_ns, 1_500);
}

#[test]
fn annotation_chunk_rollover_preserves_raw_boundaries_and_summary_count() {
    const CHUNK_SIZE: u64 = 4_096;
    let mut storage = InMemoryWordLaneStorage {
        annotations: Vec::new(),
        summary: ChunkedMipmap::new(),
        generation: 0,
    };
    append_test_words(
        &mut storage,
        (0..CHUNK_SIZE + 4).map(|index| (index * 10, 0, index)),
    );

    let annotations = &storage.annotations;
    assert_eq!(annotations.len(), (CHUNK_SIZE + 4) as usize);
    assert_eq!(
        annotations[CHUNK_SIZE as usize - 1].end_ns,
        CHUNK_SIZE * 10,
        "the word crossing the summary chunk boundary remains exact"
    );

    let summary = &storage.summary;
    assert_eq!(summary.len(), (CHUNK_SIZE + 3) as usize);
    let records = summary.sampled_window(0, (CHUNK_SIZE + 4) * 10, 1);
    assert_eq!(
        records
            .iter()
            .map(|record| u64::from(record.count))
            .sum::<u64>(),
        CHUNK_SIZE + 3
    );
}

#[test]
fn lane_growth_has_no_cap() {
    // Not a real-world entry count (that would just make the test
    // slow) — just enough to prove there's no hidden ceiling like the
    // old `MAX_LANE_ENTRIES` silently discarding past some threshold.
    const ENTRIES: u64 = 10_000;
    let mut storage = TimestampEventLaneStorage::default();
    for timestamp_ns in 0..ENTRIES {
        storage.summary.push(&timestamp_ns);
        storage.timestamps.push(timestamp_ns);
    }
    assert_eq!(storage.timestamps.len(), ENTRIES as usize);
    assert_eq!(storage.timestamps.last(), Some(&(ENTRIES - 1)));
}

#[test]
fn collector_retains_the_complete_timeline_by_default() {
    let sink = test_collector(DerivedLanes::new());
    assert_eq!(sink.retention, DerivedDataRetention::Unlimited);
}

#[test]
fn derived_data_retention_drops_oldest_exact_entries_but_keeps_full_summary() {
    let store = DerivedLanes::new();
    let mut sink = test_collector(store.clone())
        .with_indexed_words(false)
        .with_retention(DerivedDataRetention::MaxEntries(4))
        .with_words("words");
    let wd = Watchdog::new();
    let (tx, rx) = bounded::<ChannelMessage<Word>>(2);
    tx.send(ChannelMessage::Batch(
        (0..6).map(|index| Word::new(index, index * 100)).collect(),
    ))
    .unwrap();
    drop(tx);

    run_sink(
        &mut sink,
        vec![InputPort::new_with_watchdog(rx, &wd, "viewer", "in0")],
    );

    let words = lane(&store, "words");
    let table = words.table_snapshot(10).expect("word table snapshot");
    assert_eq!(
        table.rows.iter().map(|row| row.value).collect::<Vec<_>>(),
        vec![3, 4, 5]
    );
    let storage = words.storage_snapshot();
    assert_eq!(storage.retained_items, Some(3));
    assert_eq!(
        storage.index_items,
        Some(5),
        "the summary retains the five closed words"
    );
}

#[test]
fn indexed_store_creation_failure_falls_back_to_in_memory_annotations() {
    let store = DerivedLanes::new();
    let config = LiveStoreConfig {
        hot_tail_publish_words: 0,
        ..LiveStoreConfig::default()
    };

    let _sink = test_collector(store.clone())
        .with_word_store_config(config)
        .with_words("words");

    let words = lane(&store, "words");
    assert_eq!(
        words.storage_snapshot().backing,
        CollectedLaneStorageBacking::Memory
    );
    assert!(
        words
            .query::<CollectedWordLaneQuery>()
            .unwrap()
            .indexed_lane()
            .is_none()
    );
}

#[test]
fn indexed_lane_preserves_a_batch_larger_than_one_sink_drain() {
    let store = DerivedLanes::new();
    let config = LiveStoreConfig::default();
    let mut sink = test_collector(store.clone())
        .with_word_store_config(config)
        .with_words("words");
    let word_count = DRAIN_BATCH_SIZE + 17;
    let words: Vec<_> = (0..word_count as u64)
        .map(|index| Word::new(index, index * 10))
        .collect();
    let wd = Watchdog::new();
    let (tx, rx) = bounded::<ChannelMessage<Word>>(2);
    tx.send(ChannelMessage::Batch(words)).unwrap();
    drop(tx);

    run_sink(
        &mut sink,
        vec![InputPort::new_with_watchdog(rx, &wd, "viewer", "in0")],
    );

    let indexed = lane(&store, "words")
        .query::<CollectedWordLaneQuery>()
        .and_then(|query| query.indexed_lane())
        .expect("expected indexed annotation lane");
    assert_eq!(indexed.status(), StoreStatus::Finished);
    assert_eq!(indexed.metadata().total_word_count, word_count as u64);
    let tail = indexed
        .query()
        .exact_window((word_count as u64 - 3) * 10, word_count as u64 * 10, 10)
        .unwrap();
    assert!(tail.complete);
    assert_eq!(
        tail.annotations.last().unwrap().value,
        word_count as u64 - 1
    );
}

#[test]
fn indexed_lane_failure_does_not_stop_other_collected_lanes() {
    let store = DerivedLanes::new();
    let config = LiveStoreConfig::default();
    let mut sink = test_collector(store.clone())
        .with_word_store_config(config)
        .with_words("words")
        .with_trigger("trigger");
    let wd = Watchdog::new();
    let (word_tx, word_rx) = bounded::<ChannelMessage<Word>>(4);
    word_tx
        .send(ChannelMessage::Batch(vec![
            Word::new(1, 10),
            Word::new(2, 5),
        ]))
        .unwrap();
    drop(word_tx);
    let (trigger_tx, trigger_rx) = bounded::<ChannelMessage<TimestampEvent>>(4);
    trigger_tx
        .send(ChannelMessage::Sample(TimestampEvent { timestamp_ns: 42 }))
        .unwrap();
    drop(trigger_tx);

    run_sink(
        &mut sink,
        vec![
            InputPort::new_with_watchdog(word_rx, &wd, "viewer", "in0"),
            InputPort::new_with_watchdog(trigger_rx, &wd, "viewer", "in1"),
        ],
    );

    let indexed = lane(&store, "words")
        .query::<CollectedWordLaneQuery>()
        .and_then(|query| query.indexed_lane())
        .expect("expected indexed annotation lane");
    assert!(matches!(indexed.status(), StoreStatus::Failed(_)));
    assert!(matches!(
        lane_snapshot::<TimestampEventLaneSnapshot>(&lane(&store, "trigger"), 0, 100, 10).as_ref(),
        TimestampEventLaneSnapshot::Exact(markers) if markers == &[42]
    ));
}

#[test]
fn registering_a_new_indexed_writer_replaces_the_published_query_handle() {
    let config = LiveStoreConfig::default();
    let store = DerivedLanes::new();
    let first = test_collector(store.clone())
        .with_word_store_config(config.clone())
        .with_words("words");
    let first_query = lane(&store, "words")
        .query::<CollectedWordLaneQuery>()
        .expect("first published query");

    let second = test_collector(store.clone())
        .with_word_store_config(config)
        .with_words("words");
    let second_query = lane(&store, "words")
        .query::<CollectedWordLaneQuery>()
        .expect("second published query");

    assert!(!Arc::ptr_eq(&first_query, &second_query));
    let second_indexed = second_query
        .indexed_lane()
        .expect("second indexed annotation lane");
    drop((first, second));
    assert_eq!(second_indexed.status(), StoreStatus::Cancelled);
}

#[test]
fn collector_reopens_persistent_lane_and_does_not_rewrite_incoming_words() {
    let persistent = crate::derived_word_store::PersistentStoreConfig::new([9; 32]);
    let config = LiveStoreConfig {
        persistence: Some(persistent),
        ..LiveStoreConfig::default()
    };
    let (mut writer, _) =
        IndexedAnnotationWriter::create(config.clone(), crate::DecodedBlockCacheHandle::default())
            .unwrap();
    writer
        .append_batch(&[Word::new(1, 10), Word::new(2, 20)])
        .unwrap();
    writer.finish().unwrap();
    drop(writer);

    let lanes = DerivedLanes::new();
    let mut sink = test_collector(lanes.clone())
        .with_word_store_config(config)
        .with_words("words");
    let indexed = lane(&lanes, "words")
        .query::<CollectedWordLaneQuery>()
        .and_then(|query| query.indexed_lane())
        .expect("reopened indexed annotation lane");
    let wd = Watchdog::new();
    let (tx, rx) = bounded::<ChannelMessage<Word>>(4);
    tx.send(ChannelMessage::Batch(vec![
        Word::new(99, 10),
        Word::new(100, 20),
    ]))
    .unwrap();
    drop(tx);
    run_sink(
        &mut sink,
        vec![InputPort::new_with_watchdog(rx, &wd, "viewer", "in0")],
    );

    assert_eq!(indexed.metadata().total_word_count, 2);
    assert_eq!(
        indexed.query().exact_window(0, 30, 10).unwrap().annotations[0].value,
        1
    );
}
