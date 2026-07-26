use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use serde_json::Value;

use logic_analyzer_graph_api::node::{
    LiveCaptureFeature, RuntimeBuilder, graph_node_registrations,
};
use logic_analyzer_graph_api::node_support::{
    CapturePresentation, NodeBuildContext, PortKind, ResolvedInputs, SamplingOverlayDescriptor,
    ViewerOutputControl,
};
use logic_analyzer_graph_compiler::{
    CompileCtx, CompiledGraph, GraphCompiler, LiveAnalysisSource, OutputSubscriptionPlan,
    SourceArtifactReadiness, SourceDataKind, SourceProcessOverrides,
};
use logic_analyzer_graph_nodes::test_support as nodes;
use node_graph::{GraphState, NodeGraphWidget, NodeId, Socket};
use signal_processing::{
    Annotation, CaptureChannelId, CaptureChunk, CaptureChunkWriter, CaptureSessionId,
    CollectedLaneSnapshotRequest, DerivedLanes, DigitalLaneSnapshot, NativeCaptureStore,
    NativeCaptureStoreConfig, NumberLaneSnapshot, ProcessNode, Sample, SampleBlock, SamplingEdge,
    TextLaneSnapshot, Trigger, Word,
};

struct GatedProcess {
    inner: Box<dyn ProcessNode>,
    released: Arc<AtomicBool>,
}

struct GatedBinaryBuilder {
    inner: Box<dyn RuntimeBuilder>,
    released: Arc<AtomicBool>,
}

struct InstrumentedCaptureBuilder {
    inner: Box<dyn RuntimeBuilder>,
    discovery_calls: Arc<AtomicUsize>,
    provider_build_calls: Arc<AtomicUsize>,
}

impl RuntimeBuilder for InstrumentedCaptureBuilder {
    fn is_source(&self) -> bool {
        true
    }

    fn accepted_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        self.inner.accepted_kinds(socket, state)
    }

    fn offered_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        self.inner.offered_kinds(socket, state)
    }

    fn input_port(
        &self,
        socket: &Socket,
        member_index: usize,
        state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        self.inner.input_port(socket, member_index, state, kind)
    }

    fn output_port(&self, socket: &Socket, state: &Value, kind: PortKind) -> Option<String> {
        self.inner.output_port(socket, state, kind)
    }

    fn viewer_channel_origin(&self, socket: &Socket, state: &Value) -> Option<usize> {
        self.inner.viewer_channel_origin(socket, state)
    }

    fn live_capture_feature(
        &self,
        _state: &Value,
    ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
        self.discovery_calls.fetch_add(1, Ordering::SeqCst);
        Err("replay attempted provider discovery".into())
    }

    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        self.inner.input_required(socket, state)
    }

    fn build(
        &self,
        _name: &str,
        _state: &Value,
        _resolved: &ResolvedInputs,
        _ctx: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        self.provider_build_calls.fetch_add(1, Ordering::SeqCst);
        Err("replay attempted to build the provider source".into())
    }
}

impl RuntimeBuilder for GatedBinaryBuilder {
    fn accepted_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        self.inner.accepted_kinds(socket, state)
    }

    fn offered_kinds(&self, socket: &Socket, state: &Value) -> Vec<PortKind> {
        self.inner.offered_kinds(socket, state)
    }

    fn input_port(
        &self,
        socket: &Socket,
        member_index: usize,
        state: &Value,
        kind: PortKind,
    ) -> Option<String> {
        self.inner.input_port(socket, member_index, state, kind)
    }

    fn output_port(&self, socket: &Socket, state: &Value, kind: PortKind) -> Option<String> {
        self.inner.output_port(socket, state, kind)
    }

    fn word_display_format(&self, socket: &Socket, state: &Value) -> Option<String> {
        self.inner.word_display_format(socket, state)
    }

    fn sampling_overlay(&self, state: &Value) -> Option<SamplingOverlayDescriptor> {
        self.inner.sampling_overlay(state)
    }

    fn input_required(&self, socket: &Socket, state: &Value) -> bool {
        self.inner.input_required(socket, state)
    }

    fn build(
        &self,
        name: &str,
        state: &Value,
        resolved: &ResolvedInputs,
        context: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, String> {
        let inner = self.inner.build(name, state, resolved, context)?;
        Ok(Box::new(GatedProcess {
            inner,
            released: Arc::clone(&self.released),
        }))
    }
}

impl ProcessNode for GatedProcess {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn should_stop(&self) -> bool {
        self.inner.should_stop()
    }

    fn num_inputs(&self) -> usize {
        self.inner.num_inputs()
    }

    fn num_outputs(&self) -> usize {
        self.inner.num_outputs()
    }

    fn input_schema(&self) -> Vec<signal_processing::PortSchema> {
        self.inner.input_schema()
    }

    fn output_schema(&self) -> Vec<signal_processing::PortSchema> {
        self.inner.output_schema()
    }

    fn work(
        &mut self,
        inputs: &[signal_processing::InputPort],
        outputs: &[signal_processing::OutputPort],
    ) -> signal_processing::WorkResult<usize> {
        while !self.released.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        self.inner.work(inputs, outputs)
    }
}

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
    let source_name = nodes::node_name("org.logicconduit.graph-node.sigrok-file-source/v1");
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
        ("Binary Decoder", "Words"),
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

    assert_eq!(compiled.nodes.len(), 11);
    assert_eq!(compiled.edges.len(), 30);

    let spi_sampling = compiled
        .sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "SPI Decoder")
        .expect("SPI decoder should expose a sampling overlay");
    assert_eq!(spi_sampling.overlay().edge, SamplingEdge::Rising);
    assert!(!spi_sampling.overlay().sampled_channels.is_empty());
    let binary_sampling = compiled
        .sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "Binary Decoder")
        .expect("binary decoder should expose a sampling overlay");
    assert_eq!(binary_sampling.overlay().edge, SamplingEdge::Both);

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
        .find(|node| node.builder == "Binary Decoder")
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
fn built_in_uart_graph_lowers_through_the_public_compiler_facade() {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_uart_demo(&mut widget);
    let compiled = GraphCompiler::new()
        .lower(widget.graph())
        .unwrap_or_else(|errors| panic!("lower failed: {errors:?}"));

    assert_eq!(compiled.nodes.len(), 4);
    assert_eq!(compiled.edges.len(), 4);
    assert!(
        compiled
            .nodes
            .iter()
            .any(|node| node.builder == "Test UART Source")
    );
    assert!(
        compiled
            .nodes
            .iter()
            .any(|node| node.builder == "UART Decoder")
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

    for builder in ["Sigrok File Source", "SPI Decoder", "Binary Decoder"] {
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
        .unwrap_or_else(|| panic!("binary decoder word adapter was not published"))
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
fn built_in_live_analysis_matches_finalized_replay_without_provider_operations() {
    const CHUNKS: u64 = 48;
    const SAMPLES_PER_CHUNK: u64 = 128;

    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    let source_node = nodes::build_live_binary_test(&mut widget);
    select_output(&mut widget, "Binary Decoder", "Words");
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
    let release_decoder = Arc::new(AtomicBool::new(false));
    live_compiler.insert_test_builder(
        nodes::node_name("org.logicconduit.graph-node.binary-decoder/v1"),
        Box::new(GatedBinaryBuilder {
            inner: nodes::node_builder("org.logicconduit.graph-node.binary-decoder/v1"),
            released: Arc::clone(&release_decoder),
        }),
    );

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
    let processed_while_capture_finished = live_run
        .progress()
        .into_iter()
        .find_map(|(node, items)| (node == source_node).then_some(items))
        .unwrap_or(0);
    assert!(
        processed_while_capture_finished < committed_samples,
        "gated analysis unexpectedly kept up with acquisition"
    );
    let finalized = store.finalize().unwrap();
    release_decoder.store(true, Ordering::Release);
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
    let discovery_calls = Arc::new(AtomicUsize::new(0));
    let provider_build_calls = Arc::new(AtomicUsize::new(0));
    let mut replay_compiler = GraphCompiler::new();
    replay_compiler.set_output_subscriptions(subscriptions);
    replay_compiler.insert_test_builder(
        nodes::node_name("org.logicconduit.graph-node.test-capture-source/v1"),
        Box::new(InstrumentedCaptureBuilder {
            inner: nodes::node_builder("org.logicconduit.graph-node.test-capture-source/v1"),
            discovery_calls: Arc::clone(&discovery_calls),
            provider_build_calls: Arc::clone(&provider_build_calls),
        }),
    );
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
    assert_eq!(discovery_calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider_build_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn built_in_uart_second_run_reuses_persistent_words() {
    let directory = tempfile::tempdir().unwrap();
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_uart_demo(&mut widget);
    select_output(&mut widget, "UART 115200 8N1", "Data");
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
                .is_some_and(|metadata| metadata.total_rows >= 6)
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
