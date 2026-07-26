mod integration_tests_support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::Pos2;

use logic_analyzer_graph_compiler::{CompileCtx, GraphCompiler, OutputSubscriptionPlan};
use logic_analyzer_processing::nodes::decoders::parallel_decoder::{ParallelDecoder, StrobeMode};
use logic_analyzer_processing::nodes::decoders::spi_decoder::{SpiDecoder, SpiMode};
use logic_analyzer_processing::nodes::logic::logic_gate::{GateOp, LogicGate};
use logic_analyzer_processing::nodes::logic::sr_latch::SrLatch;
use logic_analyzer_processing::nodes::logic::text_formatter::TextFormatter;
use logic_analyzer_processing::nodes::logic::trigger_counter::TriggerCounter;
use logic_analyzer_processing::nodes::logic::word_matcher::WordMatcher;
use logic_analyzer_processing::nodes::sinks::binary_file_writer::BinaryFileWriter;
use logic_analyzer_processing::nodes::sources::dsl_file::DslFileSource;
use logic_analyzer_processing::types::CsPolarity;
use node_graph::{NodeGraphWidget, NodeId, SocketDirection, SocketId};
use signal_processing::{CollectedLaneSnapshotRequest, Pipeline, TriggerLaneSnapshot};

use integration_tests_support as nodes;

const STARTUP_OUTPUTS: [(&str, &str); 7] = [
    ("SPI Decoder", "MOSI Bits"),
    ("SPI Decoder", "MOSI Data"),
    ("Match Start", "Match"),
    ("Match Stop", "Match"),
    ("SR Flip-Flop", "Q"),
    ("Enable Gate", "Out"),
    ("Binary Decoder", "Words"),
];

fn capture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("_captures/wipneus5.dsl")
}

fn startup_widget() -> NodeGraphWidget {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    nodes::populate_startup(&mut widget);
    widget
}

fn startup_output_subscriptions(widget: &NodeGraphWidget) -> OutputSubscriptionPlan {
    STARTUP_OUTPUTS
        .into_iter()
        .map(|(node_title, output_name)| {
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
            (node.id, output)
        })
        .collect()
}

fn configured_compiler(widget: &NodeGraphWidget) -> GraphCompiler {
    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(startup_output_subscriptions(widget));
    compiler
}

fn node_by_definition(widget: &NodeGraphWidget, definition: &str) -> NodeId {
    widget
        .graph()
        .nodes
        .values()
        .find(|node| node.def_name() == definition)
        .map(|node| node.id)
        .unwrap_or_else(|| panic!("missing node definition '{definition}'"))
}

fn configured_widget(capture: &Path, output: &Path) -> NodeGraphWidget {
    let mut widget = startup_widget();
    let source = node_by_definition(&widget, "DSL File Source");
    let formatter = node_by_definition(&widget, "String Formatter");

    let mut source_state = widget.graph().nodes[&source].state.clone();
    source_state["file"]["value"] = capture.display().to_string().into();
    assert!(widget.set_node_state(source, source_state));

    let mut formatter_state = widget.graph().nodes[&formatter].state.clone();
    formatter_state["template"]["value"] =
        format!("{}/capture_{{n:04}}.bin", output.display()).into();
    assert!(widget.set_node_state(formatter, formatter_state));
    widget
}

fn attach_matcher_tap(widget: &mut NodeGraphWidget) -> (NodeId, usize) {
    let matcher = widget
        .add_node_at("Word Matcher", Pos2::new(620.0, 600.0))
        .expect("Word Matcher is registered");
    let mut state = widget.graph().nodes[&matcher].state.clone();
    state["pattern"]["value"] = "0x0".into();
    state["mask"]["value"] = "0x0".into();
    assert!(widget.set_node_state(matcher, state));

    let decoder = node_by_definition(widget, "Binary Decoder");
    let decoder_words = widget.graph().nodes[&decoder]
        .outputs
        .iter()
        .position(|socket| socket.name == "Words")
        .expect("Binary Decoder has Words output");
    let matcher_input = widget.graph().nodes[&matcher]
        .inputs
        .iter()
        .position(|socket| socket.name == "Words" && socket.visible)
        .expect("Word Matcher has Words input");
    let matcher_output = widget.graph().nodes[&matcher]
        .outputs
        .iter()
        .position(|socket| socket.name == "Match")
        .expect("Word Matcher has Match output");
    widget.graph_mut().add_connection(
        SocketId {
            node: decoder,
            index: decoder_words,
            direction: SocketDirection::Output,
        },
        SocketId {
            node: matcher,
            index: matcher_input,
            direction: SocketDirection::Input,
        },
    );
    (matcher, matcher_output)
}

fn run_phase_one_reference(capture: &Path, output: &Path) {
    let mut pipeline = Pipeline::new().with_default_buffer_size(10_000_000);
    pipeline
        .add_process("source", DslFileSource::new(capture).unwrap())
        .unwrap();
    pipeline
        .add_process("spi", SpiDecoder::new(SpiMode::Mode0, 24, true, false))
        .unwrap();
    pipeline
        .add_process("start", WordMatcher::new(0x600081, u64::MAX))
        .unwrap();
    pipeline
        .add_process("stop", WordMatcher::new(0x600000, u64::MAX))
        .unwrap();
    pipeline.add_process("latch", SrLatch::new(false)).unwrap();
    pipeline
        .add_process("counter", TriggerCounter::new(0, 1))
        .unwrap();
    pipeline
        .add_process(
            "formatter",
            TextFormatter::new(format!("{}/capture_{{n:04}}.bin", output.display())),
        )
        .unwrap();
    pipeline
        .add_process(
            "decoder",
            ParallelDecoder::new(8, StrobeMode::AnyEdge, CsPolarity::ActiveLow),
        )
        .unwrap();
    pipeline
        .add_process("writer", BinaryFileWriter::new().with_index_csv(true))
        .unwrap();

    pipeline.connect("source", "ch7", "spi", "clk").unwrap();
    pipeline.connect("source", "ch8", "spi", "cs").unwrap();
    pipeline.connect("source", "ch6", "spi", "mosi").unwrap();
    pipeline
        .connect("spi", "mosi_words", "start", "words")
        .unwrap();
    pipeline
        .connect("spi", "mosi_words", "stop", "words")
        .unwrap();
    pipeline
        .connect("start", "trigger", "latch", "set")
        .unwrap();
    pipeline
        .connect("stop", "trigger", "latch", "reset")
        .unwrap();
    pipeline
        .connect("latch", "q", "decoder", "enable_signal")
        .unwrap();
    pipeline
        .connect("start", "trigger", "counter", "trigger")
        .unwrap();
    pipeline
        .connect("counter", "count", "formatter", "value")
        .unwrap();
    pipeline
        .connect("formatter", "text", "writer", "filename")
        .unwrap();
    connect_parallel_inputs(&mut pipeline, "decoder");
    pipeline.connect("source", "ch8", "decoder", "cs").unwrap();
    pipeline
        .connect("decoder", "words", "writer", "data")
        .unwrap();
    pipeline.build().unwrap().wait();
}

fn run_current_reference(capture: &Path, output: &Path) {
    let mut pipeline = Pipeline::new().with_default_buffer_size(10_000_000);
    pipeline
        .add_process("source", DslFileSource::new(capture).unwrap())
        .unwrap();
    pipeline
        .add_process("spi", SpiDecoder::new(SpiMode::Mode0, 24, true, false))
        .unwrap();
    pipeline
        .add_process("start", WordMatcher::new(0x600081, u64::MAX))
        .unwrap();
    pipeline
        .add_process("stop", WordMatcher::new(0x600000, u64::MAX))
        .unwrap();
    pipeline.add_process("latch", SrLatch::new(false)).unwrap();
    pipeline
        .add_process("gate", LogicGate::new(GateOp::And, 2))
        .unwrap();
    pipeline
        .add_process("counter", TriggerCounter::new(0, 1))
        .unwrap();
    pipeline
        .add_process(
            "formatter",
            TextFormatter::new(format!("{}/capture_{{n:04}}.bin", output.display())),
        )
        .unwrap();
    pipeline
        .add_process(
            "decoder",
            ParallelDecoder::new(8, StrobeMode::AnyEdge, CsPolarity::Disabled),
        )
        .unwrap();
    pipeline
        .add_process("writer", BinaryFileWriter::new().with_index_csv(true))
        .unwrap();

    pipeline.connect("source", "ch7", "spi", "clk").unwrap();
    pipeline.connect("source", "ch8", "spi", "cs").unwrap();
    pipeline.connect("source", "ch6", "spi", "mosi").unwrap();
    pipeline
        .connect("spi", "mosi_words", "start", "words")
        .unwrap();
    pipeline
        .connect("spi", "mosi_words", "stop", "words")
        .unwrap();
    pipeline
        .connect("start", "trigger", "latch", "set")
        .unwrap();
    pipeline
        .connect("stop", "trigger", "latch", "reset")
        .unwrap();
    pipeline.connect("source", "ch8", "gate", "in0").unwrap();
    pipeline.connect("latch", "q", "gate", "in1").unwrap();
    pipeline
        .connect("gate", "out", "decoder", "enable_signal")
        .unwrap();
    pipeline
        .connect("start", "trigger", "counter", "trigger")
        .unwrap();
    pipeline
        .connect("counter", "count", "formatter", "value")
        .unwrap();
    pipeline
        .connect("formatter", "text", "writer", "filename")
        .unwrap();
    connect_parallel_inputs(&mut pipeline, "decoder");
    pipeline
        .connect("decoder", "words", "writer", "data")
        .unwrap();
    pipeline.build().unwrap().wait();
}

fn connect_parallel_inputs(pipeline: &mut Pipeline, decoder: &str) {
    pipeline
        .connect("source", "ch10", decoder, "strobe")
        .unwrap();
    for bit in 0..8 {
        pipeline
            .connect("source", &format!("ch{bit}"), decoder, &format!("d{bit}"))
            .unwrap();
    }
}

fn binary_files(directory: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| {
            let name = entry.unwrap().file_name().into_string().unwrap();
            (name.starts_with("capture_") && name.ends_with(".bin")).then_some(name)
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn normalized_csv(directory: &Path) -> Vec<String> {
    std::fs::read_to_string(directory.join("captures.csv"))
        .unwrap()
        .lines()
        .map(|line| {
            line.split(',')
                .map(|field| field.rsplit('/').next().unwrap_or(field))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect()
}

fn assert_outputs_equal(actual: &Path, expected: &Path) {
    let actual_files = binary_files(actual);
    let expected_files = binary_files(expected);
    assert!(!expected_files.is_empty());
    assert_eq!(actual_files, expected_files);
    for name in expected_files {
        let actual = std::fs::read(actual.join(&name)).unwrap();
        let expected = std::fs::read(expected.join(&name)).unwrap();
        assert_eq!(
            blake3::hash(&actual),
            blake3::hash(&expected),
            "{name} differs"
        );
    }
    assert_eq!(normalized_csv(actual), normalized_csv(expected));
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn compiler_runtime_benchmark() {
    let capture = capture_path();
    let output = tempfile::tempdir().unwrap();
    let widget = configured_widget(&capture, output.path());
    let compiler = configured_compiler(&widget);
    let mut context = CompileCtx::default();
    let started = Instant::now();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    run.wait();
    let files = binary_files(output.path());
    eprintln!(
        "compiled graph: elapsed={:.3}s files={}",
        started.elapsed().as_secs_f64(),
        files.len()
    );
    assert!(!files.is_empty());
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn phase_one_reference_runtime_benchmark() {
    let output = tempfile::tempdir().unwrap();
    let started = Instant::now();
    run_phase_one_reference(&capture_path(), output.path());
    eprintln!(
        "phase-one reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(output.path()).is_empty());
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn current_reference_runtime_benchmark() {
    let output = tempfile::tempdir().unwrap();
    let started = Instant::now();
    run_current_reference(&capture_path(), output.path());
    eprintln!(
        "current reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(output.path()).is_empty());
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn startup_graph_runtime_benchmark() {
    compiler_runtime_benchmark();
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn live_viewer_subscription_benchmark() {
    const TARGET_POINTS: usize = 5_120;
    const END_NS: u64 = 250_000_000_000;

    let output = tempfile::tempdir().unwrap();
    let widget = configured_widget(&capture_path(), output.path());
    let compiler = configured_compiler(&widget);
    let mut context = CompileCtx::default();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    let mut generations = HashMap::new();
    let mut sampled_at = HashMap::new();
    let mut query_count = 0_u64;
    while !run.is_finished() {
        let frame_started = Instant::now();
        for lane in run
            .derived_lanes()
            .opaque_lanes()
            .into_iter()
            .filter(|lane| lane.payload().stable_id() == "org.logicconduit.word/v1")
        {
            let Some(metadata) = lane.table_metadata() else {
                continue;
            };
            if generations.get(lane.name()) == Some(&metadata.generation) {
                continue;
            }
            if lane.is_live()
                && sampled_at
                    .get(lane.name())
                    .is_some_and(|sampled: &Instant| sampled.elapsed() < Duration::from_millis(50))
            {
                continue;
            }
            sampled_at.insert(lane.name().to_owned(), Instant::now());
            generations.insert(lane.name().to_owned(), metadata.generation);
            lane.snapshot(CollectedLaneSnapshotRequest {
                start_time_ns: 0,
                end_time_ns: END_NS,
                max_items: TARGET_POINTS,
            })
            .expect("word lane supports snapshots");
            query_count += 1;
        }
        std::thread::sleep(Duration::from_millis(16).saturating_sub(frame_started.elapsed()));
    }
    run.wait();
    assert!(query_count > 0);
    assert!(!binary_files(output.path()).is_empty());
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn compiled_graph_matches_current_reference() {
    let temporary = tempfile::tempdir().unwrap();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    let capture = capture_path();
    let reference = {
        let capture = capture.clone();
        let expected = expected.clone();
        std::thread::spawn(move || run_current_reference(&capture, &expected))
    };
    let widget = configured_widget(&capture, &actual);
    let compiler = configured_compiler(&widget);
    let mut context = CompileCtx::default();
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    run.wait();
    reference.join().unwrap();

    let lanes = lanes.opaque_lanes();
    assert_eq!(lanes.len(), STARTUP_OUTPUTS.len());
    assert!(lanes.iter().any(|lane| {
        lane.payload().stable_id() == "org.logicconduit.word/v1"
            && lane
                .table_metadata()
                .is_some_and(|metadata| metadata.total_rows > 0)
    }));
    assert_outputs_equal(&actual, &expected);
}

#[test]
#[ignore = "requires ignored developer-local _captures/wipneus5.dsl; run with --release"]
fn live_attach_detach_preserves_writer_output() {
    let temporary = tempfile::tempdir().unwrap();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    let capture = capture_path();
    let reference = {
        let capture = capture.clone();
        let expected = expected.clone();
        std::thread::spawn(move || run_current_reference(&capture, &expected))
    };
    let mut widget = configured_widget(&capture, &actual);
    let mut compiler = configured_compiler(&widget);
    let base_subscriptions = startup_output_subscriptions(&widget);
    let mut context = CompileCtx::default();
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();

    while binary_files(&actual).is_empty() {
        assert!(!run.is_finished());
        std::thread::sleep(Duration::from_millis(200));
    }
    let (matcher, matcher_output) = attach_matcher_tap(&mut widget);
    let mut attached_subscriptions = base_subscriptions.clone();
    attached_subscriptions.subscribe(matcher, matcher_output);
    compiler.set_output_subscriptions(attached_subscriptions);
    compiler.apply_run(&mut run, widget.graph()).unwrap();

    loop {
        let observed = lanes.opaque_lanes().iter().any(|lane| {
            lane.name().contains("Word Matcher.Match")
                && lane
                    .snapshot(CollectedLaneSnapshotRequest {
                        start_time_ns: 0,
                        end_time_ns: u64::MAX,
                        max_items: usize::MAX,
                    })
                    .and_then(|snapshot| snapshot.value::<TriggerLaneSnapshot>())
                    .is_some_and(|snapshot| {
                        matches!(snapshot.as_ref(), TriggerLaneSnapshot::Exact(markers) if !markers.is_empty())
                    })
        });
        if observed {
            break;
        }
        assert!(!run.is_finished());
        std::thread::sleep(Duration::from_millis(200));
    }
    widget.graph_mut().remove_node(matcher);
    compiler.set_output_subscriptions(base_subscriptions);
    compiler.apply_run(&mut run, widget.graph()).unwrap();
    run.wait();
    reference.join().unwrap();
    assert_outputs_equal(&actual, &expected);
}
