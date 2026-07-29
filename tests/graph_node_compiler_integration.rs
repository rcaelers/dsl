mod integration_tests_support;

use logic_analyzer_graph_api::node::{RuntimeBuilder, graph_node_registrations};
use logic_analyzer_graph_api::node_support::{CapturePresentation, ViewerOutputControl};
use logic_analyzer_graph_compiler::{
    CompileCtx, CompiledGraph, GraphCompiler, LiveAnalysisSource, OutputSubscriptionPlan,
    SourceArtifactReadiness, SourceDataKind, SourceProcessOverrides,
};
use node_graph::{GraphState, NodeGraphWidget, NodeId};
use signal_processing::{
    Annotation, CaptureChannelId, CaptureChunk, CaptureChunkWriter, CaptureSessionId,
    CollectedLaneSnapshotRequest, DerivedLanes, DigitalLaneSnapshot, NativeCaptureStore,
    NativeCaptureStoreConfig, NumberLaneSnapshot, Sample, SampleBlock, SamplingEdge,
    TextLaneSnapshot, Trigger, Word,
};

use integration_tests_support as nodes;

fn selected_outputs(graph: &GraphState) -> Vec<(NodeId, usize)> {
    let builders: std::collections::HashMap<String, Box<dyn RuntimeBuilder>> =
        graph_node_registrations()
            .into_iter()
            .filter_map(|registration| {
                registration
                    .builder()
                    .map(|builder| (registration.name().to_owned(), builder))
            })
            .collect();
    graph
        .nodes
        .iter()
        .flat_map(|(&node_id, node)| {
            let builder = builders.get(node.def_name());
            node.outputs
                .iter()
                .enumerate()
                .filter_map(move |(index, output)| {
                    let builder = builder?;
                    let ViewerOutputControl::Selectable {
                        default_selected, ..
                    } = builder.viewer_output_control(output, &node.state)?
                    else {
                        return None;
                    };
                    let selected = output
                        .extensions
                        .get("show_in_view")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(default_selected);
                    selected.then_some((node_id, index))
                })
        })
        .collect()
}

#[test]
fn binary_decoder_demo_fixture_lowers_with_built_in_nodes() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::build_binary_decoder_demo(&mut widget);
    let source_name = nodes::node_name("org.logicconduit.graph-node.sources.sigrok-file-source/v1");
    let compiler = GraphCompiler::new();
    let selected_nodes = selected_outputs(widget.graph())
        .into_iter()
        .map(|(node, _)| node)
        .collect::<Vec<_>>();
    let raw_channels = selected_nodes
        .iter()
        .filter(|node| widget.graph().nodes[node].def_name() == source_name)
        .count();
    let derived_lanes = selected_nodes
        .iter()
        .filter(|node| widget.graph().nodes[node].def_name() != source_name)
        .count();
    assert_eq!(raw_channels, 11);
    assert_eq!(derived_lanes, 0);

    let preview = compiler
        .discover_capture_presentation(widget.graph())
        .unwrap()
        .expect("demo source should provide a pre-run capture preview");
    let CapturePresentation::InMemory {
        signals: preview, ..
    } = preview.presentation
    else {
        panic!("demo source should provide an in-memory presentation");
    };
    assert_eq!(preview.len(), 10);
    assert_eq!(preview.first().unwrap().name, "Ch 0");
    assert_eq!(preview.last().unwrap().name, "Ch 10");
    assert_eq!(
        preview.last().unwrap().transitions.last().unwrap().0,
        59_999_000.0
    );

    let compiled = compiler
        .lower(widget.graph())
        .expect("demo should lower cleanly");
    assert_eq!(widget.graph().nodes.len(), 9);
    assert_eq!(compiled.nodes.len(), 8);
}

#[test]
fn event_controls_demo_fixture_loads_lowers_and_executes() {
    let graph: GraphState =
        serde_json::from_str(include_str!("../graphs/event_controls_demo.json"))
            .expect("event-controls demo should deserialize");
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    widget.set_graph(graph);

    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(
        (1..=5)
            .map(|node| (NodeId(node), 0))
            .collect::<OutputSubscriptionPlan>(),
    );
    let compiled = compiler
        .lower(widget.graph())
        .expect("event-controls demo should lower");
    assert_eq!(widget.graph().nodes.len(), 6);
    assert!(
        compiled
            .nodes
            .iter()
            .any(|node| node.builder == "Edge Detector")
    );
    assert!(
        compiled
            .nodes
            .iter()
            .any(|node| node.builder == "Event Gate")
    );
    assert_eq!(
        compiled
            .nodes
            .iter()
            .filter(|node| node.builder == "Event Control")
            .count(),
        2
    );
    let collector = compiled
        .nodes
        .iter()
        .find(|node| node.data_collector && node.resolved.member_count(0) == 5)
        .expect("the selected demo outputs should lower to a data collector");
    assert!(collector.data_collector);

    let mut context = CompileCtx::default();
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .expect("event-controls demo should start");
    run.wait();

    let lane_names = lanes
        .opaque_lanes()
        .into_iter()
        .map(|lane| lane.name().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(lane_names.len(), 5);
    assert!(
        lane_names
            .iter()
            .any(|name| name.contains("Qualified Strobe"))
    );
    assert!(
        lane_names
            .iter()
            .any(|name| name.contains("Automatic Rearm"))
    );
    assert!(lane_names.iter().any(|name| name.contains("Manual Rearm")));
}

#[test]
fn built_in_startup_graph_lowers_with_explicit_subscriptions() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_startup(&mut widget);
    for (node, output) in [
        ("SPI Decoder", "MOSI Bits"),
        ("SPI Decoder", "MOSI Data"),
        ("Match Start", "Match"),
        ("Match Stop", "Match"),
        ("SR Flip-Flop", "Q"),
        ("Enable Gate", "Out"),
        ("Parallel Decoder", "Words"),
    ] {
        select_output(&mut widget, node, output);
    }

    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(
        selected_outputs(widget.graph())
            .into_iter()
            .collect::<OutputSubscriptionPlan>(),
    );
    let compiled = compiler
        .lower(widget.graph())
        .unwrap_or_else(|errors| panic!("lower failed: {errors:?}"));

    assert_eq!(compiled.nodes.len(), 12);
    assert_eq!(compiled.edges.len(), 32);

    let spi_sampling = compiled
        .sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "SPI Decoder")
        .expect("SPI decoder should expose a sampling overlay");
    assert_eq!(spi_sampling.overlay().edge, SamplingEdge::Rising);
    assert!(!spi_sampling.overlay().sampled_channels.is_empty());
    let parallel_sampling = compiled
        .sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "Parallel Decoder")
        .expect("parallel decoder should expose a sampling overlay");
    assert_eq!(parallel_sampling.overlay().edge, SamplingEdge::Both);

    let collector = compiled
        .nodes
        .iter()
        .find(|node| node.data_collector && node.resolved.member_count(0) == 7)
        .expect("explicit subscriptions should produce a seven-lane collector");
    let lanes = collector.resolved.members(0);
    assert!(lanes.iter().any(|(_, input)| {
        input.kind == logic_analyzer_graph_api::node_support::PortKind::of::<Word>()
            && input.source == "SPI Decoder.MOSI Bits"
    }));
    assert!(lanes.iter().any(|(_, input)| {
        input.kind == logic_analyzer_graph_api::node_support::PortKind::of::<Trigger>()
            && input.source == "Match Start.Match"
    }));

    let spi = compiled
        .nodes
        .iter()
        .find(|node| node.builder == "SPI Decoder")
        .unwrap();
    let decoder = compiled
        .nodes
        .iter()
        .find(|node| node.builder == "Parallel Decoder")
        .unwrap();
    assert_eq!(
        spi.resolved.kind(0),
        Some(logic_analyzer_graph_api::node_support::PortKind::of::<Sample>())
    );
    assert_eq!(
        decoder.resolved.kind(0),
        Some(logic_analyzer_graph_api::node_support::PortKind::of::<
            SampleBlock,
        >())
    );
}

#[test]
fn built_in_binary_demo_executes_and_publishes_sampling_and_latch_data() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::build_binary_decoder_demo(&mut widget);
    for (node, output) in [
        ("SR Flip-Flop", "Q"),
        ("Parallel Enable Gate", "Out"),
        ("Match Start 0x9A", "Match"),
        ("Match Stop 0xDE", "Match"),
        ("SPI Decoder", "MOSI Bits"),
        ("SPI Decoder", "MOSI Data"),
        ("SPI Decoder", "MISO Bits"),
        ("SPI Decoder", "MISO Data"),
        ("Parallel Decoder", "Words"),
        ("Counter", "Count"),
        ("String Formatter", "Text"),
    ] {
        select_output(&mut widget, node, output);
    }

    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(selected_outputs(widget.graph()).into_iter().collect());
    let compiled = compiler.lower(widget.graph()).unwrap();
    let mut context = logic_analyzer_graph_compiler::CompileCtx::default();
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    let sampling = context.take_sampling_overlays();
    run.wait();

    for builder in ["Sigrok File Source", "SPI Decoder", "Parallel Decoder"] {
        assert!(compiled.nodes.iter().any(|node| node.builder == builder));
    }

    let enable = sampling
        .iter()
        .find(|candidate| candidate.node_title() == "Parallel Decoder")
        .and_then(|candidate| candidate.overlay().activities.first())
        .expect("parallel decoder should publish its derived enable activity");
    assert!(enable.is_active_at(800_000_000));
    assert!(!enable.is_active_at(1_200_000_000));

    let q = lanes
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.name() == "SR Flip-Flop.Q")
        .expect("latch output should be collected");
    let snapshot = q
        .snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: u64::MAX,
            max_items: usize::MAX,
        })
        .and_then(|snapshot| snapshot.value::<DigitalLaneSnapshot>())
        .expect("latch output should publish a digital snapshot");
    let DigitalLaneSnapshot::Exact { samples, .. } = snapshot.as_ref() else {
        panic!("latch output should retain exact digital data");
    };
    assert_eq!(samples.len(), 25);
    assert!(
        samples
            .iter()
            .enumerate()
            .all(|(index, sample)| sample.value == !index.is_multiple_of(2))
    );

    for (suffix, expected_kind) in [(".Count", "number"), (".Text", "text")] {
        let lane = lanes
            .opaque_lanes()
            .into_iter()
            .find(|lane| lane.name().ends_with(suffix))
            .unwrap_or_else(|| panic!("missing {suffix} lane"));
        let snapshot = lane
            .snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 0,
                end_time_ns: u64::MAX,
                max_items: usize::MAX,
            })
            .unwrap();
        let changes = snapshot
            .value::<NumberLaneSnapshot>()
            .map(|snapshot| match snapshot.as_ref() {
                NumberLaneSnapshot::Exact(samples) => samples.len(),
                NumberLaneSnapshot::Activity(_) => 0,
            })
            .or_else(|| {
                snapshot
                    .value::<TextLaneSnapshot>()
                    .map(|snapshot| match snapshot.as_ref() {
                        TextLaneSnapshot::Exact(samples) => samples.len(),
                        TextLaneSnapshot::Activity(_) => 0,
                    })
            })
            .unwrap_or_else(|| panic!("{suffix} should publish a typed {expected_kind} snapshot"));
        assert!(changes > 1, "{suffix} should contain changes");
    }
}

fn select_output(widget: &mut NodeGraphWidget, node_title: &str, output_name: &str) {
    let node = widget
        .graph()
        .nodes
        .values()
        .find(|node| node.title == node_title)
        .unwrap_or_else(|| panic!("missing node '{node_title}'"));
    let output = node
        .outputs
        .iter()
        .position(|output| output.name == output_name)
        .unwrap_or_else(|| panic!("missing output '{node_title}.{output_name}'"));
    let node = node.id;
    widget.graph_mut().nodes.get_mut(&node).unwrap().outputs[output]
        .extensions
        .insert("show_in_view".to_owned(), serde_json::json!(true));
}

fn live_analysis_chunk(
    session_id: CaptureSessionId,
    channels: &[CaptureChannelId],
    sequence: u64,
    start_sample: u64,
    sample_count: u64,
) -> CaptureChunk {
    let bit_offset = (sequence % 7) as u8;
    let bit_count = sample_count as usize * channels.len();
    let mut bytes = vec![0_u8; (usize::from(bit_offset) + bit_count).div_ceil(8)];
    for relative in 0..sample_count {
        let sample = start_sample + relative;
        for channel in 0..channels.len() {
            let value = match channel {
                0 => !sample.is_multiple_of(2),
                1 => !(sample / 2).is_multiple_of(2),
                _ => false,
            };
            if value {
                let bit = usize::from(bit_offset) + relative as usize * channels.len() + channel;
                bytes[bit / 8] |= 1 << (bit % 8);
            }
        }
    }
    CaptureChunk::packed_lsb_first(
        session_id,
        sequence,
        start_sample,
        sample_count,
        channels.to_vec(),
        bytes,
        bit_offset,
    )
    .unwrap()
}

fn captured_words(lanes: &DerivedLanes) -> Vec<Annotation> {
    lanes
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.payload().stable_id() == "org.logicconduit.word/v1")
        .and_then(|lane| lane.table_snapshot(1_000_000))
        .map(|snapshot| {
            snapshot
                .rows
                .into_iter()
                .map(|row| Annotation {
                    start_ns: row.start_time_ns,
                    end_ns: row.end_time_ns,
                    value: row.value,
                    payload: row.payload,
                })
                .collect()
        })
        .unwrap_or_else(|| panic!("parallel decoder word adapter was not published"))
}

fn annotation_bytes(annotations: &[Annotation]) -> Vec<u8> {
    annotations
        .iter()
        .flat_map(|annotation| {
            [
                annotation.start_ns.to_le_bytes(),
                annotation.end_ns.to_le_bytes(),
                annotation.value.to_le_bytes(),
            ]
            .into_iter()
            .flatten()
        })
        .collect()
}

#[test]
fn built_in_live_analysis_matches_finalized_replay_using_source_override() {
    const CHUNKS: u64 = 48;
    const SAMPLES_PER_CHUNK: u64 = 128;

    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    let source_node = nodes::build_live_binary_test(&mut widget);
    select_output(&mut widget, "Parallel Decoder", "Words");
    let subscriptions = selected_outputs(widget.graph())
        .into_iter()
        .collect::<OutputSubscriptionPlan>();

    let mut live_compiler = GraphCompiler::new();
    live_compiler.set_output_subscriptions(subscriptions.clone());
    let captured_feature = live_compiler
        .discover_live_capture_feature(widget.graph())
        .unwrap()
        .expect("test graph has a live capture feature");
    assert_eq!(captured_feature.source_node(), source_node);
    let graph_source_factory = captured_feature.graph_source_factory();

    let channels = captured_feature.channels().to_vec();
    let session_id = CaptureSessionId::new(0x4c49_5645);
    let directory = tempfile::tempdir().unwrap();
    let descriptor =
        signal_processing::CaptureStoreDescriptor::new(session_id, channels.clone()).unwrap();
    let (store, mut writer) =
        NativeCaptureStore::create(NativeCaptureStoreConfig::new(directory.path(), descriptor))
            .unwrap();

    let source = graph_source_factory
        .create(Box::new(store.open_cursor().unwrap()))
        .unwrap();
    let mut live_context = CompileCtx::default();
    let live_lanes = live_context.derived_lanes().clone();
    let mut live_run = live_compiler
        .start_live_analysis(
            widget.graph(),
            &mut live_context,
            LiveAnalysisSource {
                source_node,
                process: source,
            },
        )
        .unwrap();
    let readiness = live_run.source_readiness().snapshot();
    let source_readiness = readiness
        .iter()
        .find(|readiness| readiness.source == source_node)
        .expect("materialized live source publishes readiness");
    assert_eq!(source_readiness.kind, SourceDataKind::Live);
    assert_eq!(
        source_readiness.preload,
        SourceArtifactReadiness::Unsupported
    );
    assert_eq!(source_readiness.cache, SourceArtifactReadiness::Pending);
    assert_eq!(source_readiness.index, SourceArtifactReadiness::Pending);
    assert_eq!(source_readiness.data, SourceArtifactReadiness::Available);

    for sequence in 0..CHUNKS {
        writer
            .append(live_analysis_chunk(
                session_id,
                &channels,
                sequence,
                sequence * SAMPLES_PER_CHUNK,
                SAMPLES_PER_CHUNK,
            ))
            .unwrap();
    }
    writer.finish().unwrap();
    drop(writer);
    let committed_samples = CHUNKS * SAMPLES_PER_CHUNK;
    assert_eq!(store.snapshot().committed_samples, committed_samples);
    let finalized = store.finalize().unwrap();
    while !live_run.is_finished() {
        std::thread::yield_now();
    }
    let final_processed = live_run
        .progress()
        .into_iter()
        .find_map(|(node, items)| (node == source_node).then_some(items));
    live_run.wait();
    let live_words = captured_words(&live_lanes);
    assert!(!live_words.is_empty());

    let replay_source = graph_source_factory
        .create(Box::new(finalized.open_cursor().unwrap()))
        .unwrap();
    let mut replay_compiler = GraphCompiler::new();
    replay_compiler.set_output_subscriptions(subscriptions);
    let mut replay_context = CompileCtx::default();
    let replay_lanes = replay_context.derived_lanes().clone();
    let mut overrides = SourceProcessOverrides::new();
    overrides.insert(source_node, replay_source);
    let mut replay_run = replay_compiler
        .start_app_run_with_source_overrides(widget.graph(), &mut replay_context, overrides)
        .unwrap();
    replay_run.wait();

    assert_eq!(
        annotation_bytes(&live_words),
        annotation_bytes(&captured_words(&replay_lanes))
    );
    assert_eq!(final_processed, Some(committed_samples));
}

#[test]
fn built_in_binary_second_run_reuses_persistent_words() {
    let directory = tempfile::tempdir().unwrap();
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::build_binary_decoder_demo(&mut widget);
    select_output(&mut widget, "Parallel Decoder", "Words");
    let subscriptions = selected_outputs(widget.graph())
        .into_iter()
        .collect::<OutputSubscriptionPlan>();
    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(subscriptions);

    let mut first_context = CompileCtx::default();
    first_context.set_persistent_cache_directory(directory.path().to_path_buf());
    let mut first = compiler
        .start_app_run(widget.graph(), &mut first_context)
        .unwrap();
    first.wait();
    assert!(!first.persistent_cache_configs().is_empty());
    drop((first, first_context));

    let mut second_context = CompileCtx::default();
    second_context.set_persistent_cache_directory(directory.path().to_path_buf());
    let lanes = second_context.derived_lanes().clone();
    let mut second = compiler
        .start_app_run(widget.graph(), &mut second_context)
        .unwrap();
    second.wait();

    assert!(lanes.opaque_lanes().iter().any(|lane| {
        lane.payload().stable_id() == "org.logicconduit.word/v1"
            && lane
                .table_metadata()
                .is_some_and(|metadata| metadata.total_rows > 0)
    }));
}

#[test]
fn built_in_graph_json_round_trip_compiles_identically() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_startup(&mut widget);
    let compiler = GraphCompiler::new();
    let original = compiler.lower(widget.graph()).expect("original lowers");

    let json = serde_json::to_string(widget.graph()).expect("graph serializes");
    let restored_state: GraphState = serde_json::from_str(&json).expect("graph deserializes");
    let mut restored = NodeGraphWidget::new(nodes::build_registry());
    restored.set_graph(restored_state);

    let reloaded = compiler.lower(restored.graph()).expect("restored lowers");

    assert_eq!(original.nodes.len(), reloaded.nodes.len());
    for (before, after) in original.nodes.iter().zip(&reloaded.nodes) {
        assert_eq!(before.id, after.id);
        assert_eq!(before.builder, after.builder);
        assert_eq!(before.state, after.state);
    }
    assert_eq!(compiled_edges(&original), compiled_edges(&reloaded));
}

fn compiled_edges(compiled: &CompiledGraph) -> Vec<String> {
    let mut edges = compiled
        .edges
        .iter()
        .map(|edge| {
            format!(
                "n{}:{} -> n{}:{} ({})",
                edge.from.0.0, edge.from.1, edge.to.0.0, edge.to.1, edge.buffer
            )
        })
        .collect::<Vec<_>>();
    edges.sort();
    edges
}
