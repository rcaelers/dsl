#[path = "../tests/integration_tests_support/mod.rs"]
mod integration_tests_support;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    ("Parallel Decoder", "Words"),
];
const VALIDATION_OUTPUTS: [(&str, &str); 1] = [("SPI Decoder", "MOSI Bits")];
const CHECKED_IN_GRAPH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/graphs/spi_controlled_decode.json"
);

#[derive(Debug, Eq, PartialEq)]
struct OutputFingerprint {
    name: String,
    size: u64,
    hash: String,
}

#[derive(Clone, Copy, Debug)]
struct ResourceUsage {
    user_seconds: f64,
    system_seconds: f64,
    peak_rss_bytes: u64,
}

#[cfg(unix)]
fn resource_usage() -> Option<ResourceUsage> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `usage` points to writable storage for one `libc::rusage`,
    // which `getrusage` initializes when it succeeds.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful call above initialized the value.
    let usage = unsafe { usage.assume_init() };
    let timeval_seconds =
        |time: libc::timeval| time.tv_sec as f64 + time.tv_usec as f64 / 1_000_000.0;
    #[cfg(target_os = "macos")]
    let peak_rss_bytes = usage.ru_maxrss.max(0) as u64;
    #[cfg(not(target_os = "macos"))]
    let peak_rss_bytes = (usage.ru_maxrss.max(0) as u64).saturating_mul(1024);
    Some(ResourceUsage {
        user_seconds: timeval_seconds(usage.ru_utime),
        system_seconds: timeval_seconds(usage.ru_stime),
        peak_rss_bytes,
    })
}

#[cfg(not(unix))]
fn resource_usage() -> Option<ResourceUsage> {
    None
}

fn resource_delta(
    before: Option<ResourceUsage>,
    after: Option<ResourceUsage>,
    wall: Duration,
) -> serde_json::Value {
    let (Some(before), Some(after)) = (before, after) else {
        return serde_json::json!({
            "cpu_user_s": null,
            "cpu_system_s": null,
            "average_cpu_cores": null,
            "peak_rss_bytes": null,
        });
    };
    let user_seconds = (after.user_seconds - before.user_seconds).max(0.0);
    let system_seconds = (after.system_seconds - before.system_seconds).max(0.0);
    let average_cpu_cores = (user_seconds + system_seconds) / wall.as_secs_f64().max(f64::EPSILON);
    serde_json::json!({
        "cpu_user_s": user_seconds,
        "cpu_system_s": system_seconds,
        "average_cpu_cores": average_cpu_cores,
        "peak_rss_bytes": after.peak_rss_bytes,
    })
}

fn startup_widget() -> NodeGraphWidget {
    let mut widget = NodeGraphWidget::new(nodes::build_registry());
    widget
        .load_from_path(CHECKED_IN_GRAPH)
        .expect("checked-in SPI controlled decode graph should load");
    widget
}

fn startup_output_subscriptions(widget: &NodeGraphWidget) -> OutputSubscriptionPlan {
    output_subscriptions(widget, &STARTUP_OUTPUTS)
}

fn output_subscriptions(
    widget: &NodeGraphWidget,
    outputs: &[(&str, &str)],
) -> OutputSubscriptionPlan {
    outputs
        .iter()
        .map(|&(node_title, output_name)| {
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
    let mut subscriptions = startup_output_subscriptions(widget);
    subscriptions.subscribe_sampling_overlay(node_by_definition(widget, "SPI Decoder"));
    subscriptions.subscribe_sampling_overlay(node_by_definition(widget, "Parallel Decoder"));
    compiler.set_output_subscriptions(subscriptions);
    compiler
}

fn validation_compiler(widget: &NodeGraphWidget) -> GraphCompiler {
    let mut compiler = GraphCompiler::new();
    compiler.set_output_subscriptions(output_subscriptions(widget, &VALIDATION_OUTPUTS));
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

    let decoder = node_by_definition(widget, "Parallel Decoder");
    let decoder_words = widget.graph().nodes[&decoder]
        .outputs
        .iter()
        .position(|socket| socket.name == "Words")
        .expect("Parallel Decoder has Words output");
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

fn temporary_workspace() -> tempfile::TempDir {
    let workspace = tempfile::Builder::new()
        .prefix("logic-conduit-compiler-capture-")
        .tempdir()
        .expect("temporary validation workspace should be available");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .expect("repository root should be available");
    let workspace_path = workspace
        .path()
        .canonicalize()
        .expect("temporary validation workspace should be available");
    assert!(
        !workspace_path.starts_with(&repository),
        "temporary validation workspace '{}' must be outside the repository",
        workspace_path.display()
    );
    workspace
}

fn compile_context(workspace: &Path) -> CompileCtx {
    let cache = workspace.join("derived-cache");
    std::fs::create_dir_all(&cache).expect("temporary derived cache should be available");
    let mut context = CompileCtx::default();
    context.set_persistent_cache_directory(cache);
    context
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

fn normalized_csv(directory: &Path) -> Vec<u8> {
    std::fs::read_to_string(directory.join("captures.csv"))
        .unwrap()
        .lines()
        .map(|line| {
            line.split(',')
                .map(|field| field.rsplit(['/', '\\']).next().unwrap_or(field))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

fn fingerprint_file(path: &Path) -> (u64, String) {
    let mut file = std::fs::File::open(path).expect("output file should be readable");
    let size = file
        .metadata()
        .expect("output metadata should be readable")
        .len();
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .expect("output file should remain readable");
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    (size, hasher.finalize().to_hex().to_string())
}

fn output_manifest(directory: &Path) -> Vec<OutputFingerprint> {
    let mut entries = std::fs::read_dir(directory)
        .expect("output directory should be readable")
        .filter_map(|entry| {
            let entry = entry.expect("output entry should be readable");
            let file_type = entry
                .file_type()
                .expect("output entry type should be readable");
            file_type.is_file().then_some(entry)
        })
        .map(|entry| {
            let name = entry
                .file_name()
                .into_string()
                .expect("output names should be UTF-8");
            let (size, hash) = if name == "captures.csv" {
                let bytes = normalized_csv(directory);
                (
                    bytes.len() as u64,
                    blake3::hash(&bytes).to_hex().to_string(),
                )
            } else {
                fingerprint_file(&entry.path())
            };
            OutputFingerprint { name, size, hash }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

fn manifest_fingerprint(manifest: &[OutputFingerprint]) -> String {
    let mut hasher = blake3::Hasher::new();
    for output in manifest {
        for field in [output.name.as_bytes(), output.hash.as_bytes()] {
            hasher.update(&(field.len() as u64).to_le_bytes());
            hasher.update(field);
        }
        hasher.update(&output.size.to_le_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn output_manifest_report(manifest: &[OutputFingerprint]) -> serde_json::Value {
    let total_bytes = manifest
        .iter()
        .map(|output| output.size)
        .fold(0_u64, u64::saturating_add);
    serde_json::json!({
        "fingerprint": manifest_fingerprint(manifest),
        "total_bytes": total_bytes,
        "files": manifest
            .iter()
            .map(|output| serde_json::json!({
                "name": output.name,
                "size": output.size,
                "blake3": output.hash,
            }))
            .collect::<Vec<_>>(),
    })
}

fn cache_key_hex(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn derived_storage_report(run: &logic_analyzer_graph_compiler::LiveRun) -> serde_json::Value {
    let mut backing_counts = BTreeMap::<String, u64>::new();
    let mut retained_items = 0_u64;
    let mut resident_bytes = 0_u64;
    let mut stored_bytes = 0_u64;
    let mut index_items = 0_u64;
    let mut index_bytes = 0_u64;
    let lanes = run
        .derived_lanes()
        .opaque_lanes()
        .into_iter()
        .map(|lane| {
            let storage = lane.storage_snapshot();
            *backing_counts
                .entry(format!("{:?}", storage.backing))
                .or_default() += 1;
            retained_items = retained_items.saturating_add(storage.retained_items.unwrap_or(0));
            resident_bytes = resident_bytes.saturating_add(storage.resident_bytes.unwrap_or(0));
            stored_bytes = stored_bytes.saturating_add(storage.stored_bytes.unwrap_or(0));
            index_items = index_items.saturating_add(storage.index_items.unwrap_or(0));
            index_bytes = index_bytes.saturating_add(storage.index_bytes.unwrap_or(0));
            serde_json::json!({
                "name": lane.name(),
                "payload": lane.payload().stable_id(),
                "backing": format!("{:?}", storage.backing),
                "retained_items": storage.retained_items,
                "resident_bytes": storage.resident_bytes,
                "stored_bytes": storage.stored_bytes,
                "index_items": storage.index_items,
                "index_bytes": storage.index_bytes,
            })
        })
        .collect::<Vec<_>>();

    let mut seen = HashSet::new();
    let mut persistent_data_bytes = 0_u64;
    let mut persistent_index_bytes = 0_u64;
    let mut persistent_total_bytes = 0_u64;
    let mut persistent_words = 0_u64;
    let caches = run
        .persistent_cache_configs()
        .into_iter()
        .filter(|config| seen.insert(config.cache_key))
        .map(|config| {
            let inspected = signal_processing::derived_word_store::inspect_cache_entry(&config)
                .unwrap_or_else(|error| panic!("could not inspect derived cache: {error}"));
            if let Some(snapshot) = inspected {
                persistent_data_bytes = persistent_data_bytes.saturating_add(snapshot.data_bytes);
                persistent_index_bytes =
                    persistent_index_bytes.saturating_add(snapshot.index_bytes);
                persistent_total_bytes =
                    persistent_total_bytes.saturating_add(snapshot.total_bytes);
                persistent_words = persistent_words.saturating_add(snapshot.word_count);
                serde_json::json!({
                    "cache_key": cache_key_hex(&config.cache_key),
                    "total_bytes": snapshot.total_bytes,
                    "data_bytes": snapshot.data_bytes,
                    "index_bytes": snapshot.index_bytes,
                    "word_count": snapshot.word_count,
                    "block_count": snapshot.block_count,
                })
            } else {
                serde_json::json!({
                    "cache_key": cache_key_hex(&config.cache_key),
                    "missing": true,
                })
            }
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "lane_count": lanes.len(),
        "backing_counts": backing_counts,
        "retained_items": retained_items,
        "resident_bytes": resident_bytes,
        "stored_bytes": stored_bytes,
        "index_items": index_items,
        "index_bytes": index_bytes,
        "lanes": lanes,
        "persistent_cache": {
            "entry_count": caches.len(),
            "word_count": persistent_words,
            "data_bytes": persistent_data_bytes,
            "index_bytes": persistent_index_bytes,
            "total_bytes": persistent_total_bytes,
            "entries": caches,
        },
    })
}

fn print_manifest(manifest: &[OutputFingerprint]) {
    eprintln!("validated output manifest:");
    for output in manifest {
        eprintln!(
            "  name={} size={} blake3={}",
            output.name, output.size, output.hash
        );
    }
}

fn assert_outputs_equal(actual: &Path, expected: &Path) {
    let actual_manifest = output_manifest(actual);
    let expected_manifest = output_manifest(expected);
    assert!(
        expected_manifest
            .iter()
            .any(|output| output.name.starts_with("capture_") && output.name.ends_with(".bin")),
        "reference pipeline did not produce capture files"
    );
    let actual_names = actual_manifest
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>();
    let expected_names = expected_manifest
        .iter()
        .map(|output| output.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual_names, expected_names, "output names differ");
    for (actual, expected) in actual_manifest.iter().zip(&expected_manifest) {
        assert_eq!(
            actual.size, expected.size,
            "output size differs for '{}'",
            expected.name
        );
        assert_eq!(
            actual.hash, expected.hash,
            "output hash differs for '{}'",
            expected.name
        );
    }
    print_manifest(&actual_manifest);
}

fn run_validation_child(stage: &str, capture: &Path, output: &Path, workspace: Option<&Path>) {
    let mut command = Command::new(
        std::env::current_exe().expect("validation executable path should be available"),
    );
    command.arg(stage).arg(capture).arg(output);
    if let Some(workspace) = workspace {
        command.arg(workspace);
    }
    let started = Instant::now();
    eprintln!("validation stage={}", stage.trim_start_matches("__"));
    let status = command
        .status()
        .expect("validation child process should start");
    assert!(
        status.success(),
        "validation stage '{}' failed with {status}",
        stage.trim_start_matches("__")
    );
    eprintln!(
        "validation stage={} elapsed={:.3}s",
        stage.trim_start_matches("__"),
        started.elapsed().as_secs_f64()
    );
}

fn run_compiled_validation_stage(capture: &Path, output: &Path, workspace: &Path) {
    let widget = configured_widget(capture, output);
    let compiler = validation_compiler(&widget);
    let mut context = compile_context(workspace);
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    run.wait();

    let lanes = lanes.opaque_lanes();
    for (node, output) in VALIDATION_OUTPUTS {
        let expected = format!("{node}.{output}");
        assert!(
            lanes.iter().any(|lane| lane.name() == expected),
            "compiled graph did not collect subscribed output '{expected}'"
        );
    }
    assert!(lanes.iter().any(|lane| {
        lane.payload().stable_id() == "org.logicconduit.word/v1"
            && lane
                .table_metadata()
                .is_some_and(|metadata| metadata.total_rows > 0)
    }));
}

fn run_compiled_baseline_stage(capture: &Path, output: &Path, workspace: &Path, report: &Path) {
    let total_started = Instant::now();
    let graph_started = Instant::now();
    let widget = configured_widget(capture, output);
    let compiler = configured_compiler(&widget);
    let graph_load = graph_started.elapsed();

    let lower_started = Instant::now();
    let compiled = compiler
        .lower(widget.graph())
        .unwrap_or_else(|errors| panic!("checked-in graph did not lower: {errors:?}"));
    let lower = lower_started.elapsed();
    let compiled_nodes = compiled.nodes.len();
    let compiled_edges = compiled.edges.len();

    let mut context = compile_context(workspace);
    let resources_before = resource_usage();
    let start_started = Instant::now();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap_or_else(|errors| panic!("checked-in graph did not start: {errors:?}"));
    let pipeline_start = start_started.elapsed();
    let execute_started = Instant::now();
    run.wait();
    let pipeline_execute = execute_started.elapsed();
    let pipeline_wall = pipeline_start.saturating_add(pipeline_execute);
    let resources = resource_delta(resources_before, resource_usage(), pipeline_wall);

    let storage_started = Instant::now();
    let storage = derived_storage_report(&run);
    let storage_inspection = storage_started.elapsed();
    let report_value = serde_json::json!({
        "schema_version": 1,
        "compiled_graph": {
            "nodes": compiled_nodes,
            "edges": compiled_edges,
        },
        "phases": {
            "graph_load_s": graph_load.as_secs_f64(),
            "graph_lower_s": lower.as_secs_f64(),
            "pipeline_start_s": pipeline_start.as_secs_f64(),
            "pipeline_execute_s": pipeline_execute.as_secs_f64(),
            "pipeline_wall_s": pipeline_wall.as_secs_f64(),
            "storage_inspection_s": storage_inspection.as_secs_f64(),
            "child_total_s": total_started.elapsed().as_secs_f64(),
        },
        "resources": resources,
        "derived_storage": storage,
    });
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_value).expect("baseline report should serialize"),
    )
    .expect("baseline report should be writable");
}

fn run_cancellation_stage(capture: &Path, output: &Path, workspace: &Path, report: &Path) {
    let widget = configured_widget(capture, output);
    let compiler = configured_compiler(&widget);
    let mut context = compile_context(workspace);
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap_or_else(|errors| panic!("cancellation graph did not start: {errors:?}"));
    let warmup_started = Instant::now();
    while !run.is_finished()
        && !run.progress().iter().any(|(_, progress)| *progress > 0)
        && warmup_started.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        !run.is_finished(),
        "pipeline finished before cancellation probe"
    );
    let progress_before_stop = run
        .progress()
        .into_iter()
        .map(|(_, progress)| progress)
        .fold(0_u64, u64::saturating_add);
    let stop_started = Instant::now();
    run.stop();
    while !run.is_finished() && stop_started.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(1));
    }
    let stop_latency = stop_started.elapsed();
    assert!(
        run.is_finished(),
        "pipeline cancellation exceeded 10 seconds"
    );
    run.wait();
    let report_value = serde_json::json!({
        "warmup_s": warmup_started.elapsed().as_secs_f64(),
        "progress_before_stop": progress_before_stop,
        "stop_latency_ms": stop_latency.as_secs_f64() * 1_000.0,
    });
    std::fs::write(
        report,
        serde_json::to_vec_pretty(&report_value).expect("cancellation report should serialize"),
    )
    .expect("cancellation report should be writable");
}

fn run_measurement_child(
    stage: &str,
    capture: &Path,
    output: &Path,
    workspace: &Path,
    report: &Path,
) {
    let started = Instant::now();
    eprintln!("baseline stage={}", stage.trim_start_matches("__"));
    let status = Command::new(
        std::env::current_exe().expect("benchmark executable path should be available"),
    )
    .arg(stage)
    .arg(capture)
    .arg(output)
    .arg(workspace)
    .arg(report)
    .status()
    .expect("benchmark child process should start");
    assert!(
        status.success(),
        "baseline stage '{}' failed with {status}",
        stage.trim_start_matches("__")
    );
    eprintln!(
        "baseline stage={} elapsed={:.3}s",
        stage.trim_start_matches("__"),
        started.elapsed().as_secs_f64()
    );
}

fn sidecar_size(path: &Path, suffix: &str) -> Option<u64> {
    let name = format!(
        "{}.{suffix}",
        path.file_name().and_then(|name| name.to_str())?
    );
    std::fs::metadata(path.with_file_name(name))
        .ok()
        .map(|metadata| metadata.len())
}

fn rustc_version() -> Option<String> {
    let output = Command::new("rustc").arg("--version").output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn reference_pipeline_baseline(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    let cancellation_workspace = workspace.path().join("cancellation");
    let cancellation_output = cancellation_workspace.join("output");
    std::fs::create_dir_all(&output).unwrap();
    std::fs::create_dir_all(&cancellation_output).unwrap();
    let child_report = workspace.path().join("baseline.json");
    let cancellation_report = workspace.path().join("cancellation.json");
    run_measurement_child(
        "__baseline",
        capture,
        &output,
        workspace.path(),
        &child_report,
    );
    run_measurement_child(
        "__cancel",
        capture,
        &cancellation_output,
        &cancellation_workspace,
        &cancellation_report,
    );

    let fingerprint_started = Instant::now();
    let manifest = output_manifest(&output);
    let output_report = output_manifest_report(&manifest);
    let fingerprint_elapsed = fingerprint_started.elapsed().as_secs_f64();
    let mut report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&child_report).expect("baseline child report should exist"),
    )
    .expect("baseline child report should be valid JSON");
    let cancellation: serde_json::Value = serde_json::from_slice(
        &std::fs::read(&cancellation_report).expect("cancellation report should exist"),
    )
    .expect("cancellation report should be valid JSON");
    let source = DslFileSource::new(capture).expect("reference capture should open");
    let total_samples = source.total_samples();
    let samplerate_hz = source.samplerate_hz();
    let capture_seconds = total_samples as f64 / samplerate_hz;
    let pipeline_wall = report["phases"]["pipeline_wall_s"]
        .as_f64()
        .expect("pipeline wall time should be numeric");
    let graph_bytes = std::fs::read(CHECKED_IN_GRAPH).expect("checked-in graph should be readable");
    let capture_identity = DslFileSource::capture_cache_identity(capture)
        .expect("reference capture should have a stable identity");
    let generated_unix_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should follow the Unix epoch")
        .as_secs();
    let root = report
        .as_object_mut()
        .expect("baseline report root should be an object");
    root.insert(
        "environment".to_owned(),
        serde_json::json!({
            "generated_unix_s": generated_unix_s,
            "package_version": env!("CARGO_PKG_VERSION"),
            "rustc": rustc_version(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "logical_cpu_count": std::thread::available_parallelism().ok().map(|count| count.get()),
            "profile": "bench",
        }),
    );
    root.insert(
        "graph".to_owned(),
        serde_json::json!({
            "path": "graphs/spi_controlled_decode.json",
            "blake3": blake3::hash(&graph_bytes).to_hex().to_string(),
        }),
    );
    root.insert(
        "capture".to_owned(),
        serde_json::json!({
            "path": capture,
            "cache_identity": cache_key_hex(&capture_identity),
            "file_bytes": std::fs::metadata(capture).expect("capture metadata should be readable").len(),
            "total_samples": total_samples,
            "samplerate_hz": samplerate_hz,
            "duration_s": capture_seconds,
            "channels": source.total_probes(),
            "waveform_index_bytes": sidecar_size(capture, "idx"),
            "waveform_raw_bytes": sidecar_size(capture, "raw"),
        }),
    );
    root.insert(
        "throughput".to_owned(),
        serde_json::json!({
            "million_samples_per_s": total_samples as f64 / pipeline_wall / 1_000_000.0,
            "realtime_factor": capture_seconds / pipeline_wall,
            "target_realtime_factor": 6.0,
            "target_met": capture_seconds / pipeline_wall >= 6.0,
        }),
    );
    root.insert("outputs".to_owned(), output_report);
    root.insert("cancellation".to_owned(), cancellation);
    report["phases"]["output_fingerprint_s"] = fingerprint_elapsed.into();
    eprintln!(
        "baseline pipeline_wall_s={pipeline_wall:.3} realtime_x={:.3} output_fingerprint={}",
        capture_seconds / pipeline_wall,
        report["outputs"]["fingerprint"]
            .as_str()
            .expect("output fingerprint should be a string")
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("baseline report should serialize")
    );
}

fn compiler_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    std::fs::create_dir_all(&output).unwrap();
    let widget = configured_widget(capture, &output);
    let compiler = configured_compiler(&widget);
    let mut context = compile_context(workspace.path());
    let started = Instant::now();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    let sampling_overlays = context.take_sampling_overlays();
    run.wait();
    let execution_elapsed = started.elapsed();
    eprintln!(
        "compiled graph execution: elapsed={:.3}s",
        execution_elapsed.as_secs_f64()
    );
    let files = binary_files(&output);
    let sampling_points = sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "SPI Decoder")
        .expect("configured SPI sampling overlay should be available")
        .overlay()
        .points
        .points_in_range(0, u64::MAX)
        .len();
    let parallel_sampling = sampling_overlays
        .iter()
        .find(|candidate| candidate.node_title() == "Parallel Decoder")
        .expect("parallel sampling overlay should be available");
    let parallel_sampling_is_lazy = parallel_sampling.overlay().points.has_provider();
    let first_parallel_time = run
        .derived_lanes()
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.name().contains("Parallel Decoder.Words"))
        .and_then(|lane| lane.nearest_time_boundary(0, 1_000_000_000))
        .expect("parallel word lane should expose its first timestamp");
    let visible_start = first_parallel_time.saturating_sub(100);
    let visible_end = first_parallel_time.saturating_add(10_000);
    let parallel_sampling_is_dense = parallel_sampling
        .overlay()
        .points
        .points_in_range_with_minimum_spacing(visible_start, visible_end, 1_000_000_000)
        .is_none();
    let visible_parallel_points = parallel_sampling
        .overlay()
        .points
        .points_in_range(visible_start, visible_end)
        .len();
    eprintln!(
        "compiled graph: elapsed={:.3}s overlay_queries={:.3}s files={} sampling_points={sampling_points} parallel_lazy={parallel_sampling_is_lazy} parallel_visible_points={visible_parallel_points}",
        started.elapsed().as_secs_f64(),
        started
            .elapsed()
            .saturating_sub(execution_elapsed)
            .as_secs_f64(),
        files.len()
    );
    assert!(!files.is_empty());
    assert!(sampling_points > 0);
    assert!(parallel_sampling_is_lazy);
    assert!(parallel_sampling_is_dense);
    assert!(visible_parallel_points > 0);
}

fn phase_one_reference_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("phase-one-reference");
    std::fs::create_dir_all(&output).unwrap();
    let started = Instant::now();
    run_phase_one_reference(capture, &output);
    eprintln!(
        "phase-one reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(&output).is_empty());
}

fn current_reference_runtime_benchmark(capture: &Path) {
    let workspace = temporary_workspace();
    let output = workspace.path().join("current-reference");
    std::fs::create_dir_all(&output).unwrap();
    let started = Instant::now();
    run_current_reference(capture, &output);
    eprintln!(
        "current reference: elapsed={:.3}s",
        started.elapsed().as_secs_f64()
    );
    assert!(!binary_files(&output).is_empty());
}

fn live_viewer_subscription_benchmark(capture: &Path) {
    const TARGET_POINTS: usize = 5_120;
    const END_NS: u64 = 250_000_000_000;

    let workspace = temporary_workspace();
    let output = workspace.path().join("compiled");
    std::fs::create_dir_all(&output).unwrap();
    let widget = configured_widget(capture, &output);
    let compiler = configured_compiler(&widget);
    let mut context = compile_context(workspace.path());
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
    assert!(!binary_files(&output).is_empty());
}

fn compiled_graph_matches_current_reference(capture: &Path) {
    let temporary = temporary_workspace();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    run_validation_child("__compiled", capture, &actual, Some(temporary.path()));
    run_validation_child("__reference", capture, &expected, None);
    assert_outputs_equal(&actual, &expected);
}

fn live_attach_detach_preserves_writer_output(capture: &Path) {
    let temporary = temporary_workspace();
    let actual = temporary.path().join("compiled");
    let expected = temporary.path().join("reference");
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&expected).unwrap();
    run_validation_child("__reference", capture, &expected, None);
    eprintln!("attach validation stage=compiled");
    let mut widget = configured_widget(capture, &actual);
    let mut compiler = configured_compiler(&widget);
    let base_subscriptions = startup_output_subscriptions(&widget);
    let mut context = compile_context(temporary.path());
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
    assert_outputs_equal(&actual, &expected);
}

fn print_usage() {
    eprintln!(
        "usage: cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
         <command> <capture.dsl>\n\
         \n\
         commands:\n\
           baseline               report timing, resources, storage, and output identity\n\
           compiler-runtime       time the compiled startup graph\n\
           phase-one-runtime      time the phase-one reference pipeline\n\
           current-runtime        time the current reference pipeline\n\
           live-viewer-runtime    exercise live viewer queries while processing\n\
           validate-compiled      compare compiled and reference output\n\
           validate-live-attach   verify attach/detach preserves writer output\n\
           all                    run every command"
    );
}

fn main() {
    let arguments = std::env::args_os()
        .skip(1)
        .filter(|argument| argument != "--bench")
        .collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "__reference")
    {
        assert_eq!(arguments.len(), 3, "invalid reference-stage arguments");
        run_current_reference(Path::new(&arguments[1]), Path::new(&arguments[2]));
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__compiled")
    {
        assert_eq!(arguments.len(), 4, "invalid compiled-stage arguments");
        run_compiled_validation_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__baseline")
    {
        assert_eq!(arguments.len(), 5, "invalid baseline-stage arguments");
        run_compiled_baseline_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
            Path::new(&arguments[4]),
        );
        return;
    }
    if arguments
        .first()
        .is_some_and(|argument| argument == "__cancel")
    {
        assert_eq!(arguments.len(), 5, "invalid cancellation-stage arguments");
        run_cancellation_stage(
            Path::new(&arguments[1]),
            Path::new(&arguments[2]),
            Path::new(&arguments[3]),
            Path::new(&arguments[4]),
        );
        return;
    }
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        print_usage();
        return;
    };
    if command == "--help" || command == "-h" {
        print_usage();
        return;
    }
    let Some(capture) = arguments.next().map(PathBuf::from) else {
        print_usage();
        panic!("a capture path is required");
    };
    assert!(arguments.next().is_none(), "unexpected extra arguments");
    assert!(
        capture.is_file(),
        "capture file '{}' does not exist",
        capture.display()
    );

    match command.to_string_lossy().as_ref() {
        "baseline" => reference_pipeline_baseline(&capture),
        "compiler-runtime" => compiler_runtime_benchmark(&capture),
        "phase-one-runtime" => phase_one_reference_runtime_benchmark(&capture),
        "current-runtime" => current_reference_runtime_benchmark(&capture),
        "live-viewer-runtime" => live_viewer_subscription_benchmark(&capture),
        "validate-compiled" => compiled_graph_matches_current_reference(&capture),
        "validate-live-attach" => live_attach_detach_preserves_writer_output(&capture),
        "all" => {
            compiler_runtime_benchmark(&capture);
            phase_one_reference_runtime_benchmark(&capture);
            current_reference_runtime_benchmark(&capture);
            live_viewer_subscription_benchmark(&capture);
            compiled_graph_matches_current_reference(&capture);
            live_attach_detach_preserves_writer_output(&capture);
        }
        command => {
            print_usage();
            panic!("unknown command '{command}'");
        }
    }
}
