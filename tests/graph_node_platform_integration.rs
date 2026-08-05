mod integration_tests_support;

use egui::Pos2;

use logic_analyzer_graph_capabilities::node_support::{
    CapturePresentation, LiveCaptureEdit, SourceDataLifecycleKind,
};
use node_graph::NodeGraphWidget;
use signal_capture_session::{CaptureChannelId, CaptureDataDelivery, SimpleTriggerCondition};

use integration_tests_support::{
    GraphHarness, build_live_binary_test, build_registry, node_name, node_semantics,
};

const U3PRO16_ID: &str = "org.logicconduit.graph-node.sources.dslogic-u3pro16/v1";
const DSL_FILE_SOURCE_ID: &str = "org.logicconduit.graph-node.sources.dsl-file-source/v1";

fn select(state: &mut serde_json::Value, field: &str, value: &str) {
    state[field]["value"] = serde_json::Value::String(value.to_owned());
}

fn enable_channels(state: &mut serde_json::Value, channels: &[usize]) {
    let enabled = state["channels"]["enabled"]
        .as_array_mut()
        .expect("capture source channels are an array");
    enabled.fill(serde_json::Value::Bool(false));
    for &channel in channels {
        enabled[channel] = serde_json::Value::Bool(true);
    }
}

#[test]
fn native_hardware_source_registers_and_lowers() {
    let mut widget = NodeGraphWidget::new(build_registry());
    let source = build_live_binary_test(&mut widget);
    assert_eq!(
        widget.graph().nodes[&source].def_name(),
        node_name(U3PRO16_ID)
    );
    let decoder = widget
        .graph()
        .nodes
        .values()
        .find(|node| {
            node.def_name() == node_name("org.logicconduit.graph-node.decoders.parallel-decoder/v1")
        })
        .expect("parallel decoder should be registered");
    let words = decoder
        .outputs
        .iter()
        .position(|output| output.name == "Words")
        .expect("parallel decoder exposes words");
    let decoder = decoder.id;

    let mut compiler = GraphHarness::new();
    compiler.set_output_subscriptions([(decoder, words)].into_iter().collect());
    let compiled = compiler.lowerer().lower(widget.graph()).unwrap();
    assert!(
        compiled
            .nodes
            .iter()
            .any(|node| node.builder == node_name(U3PRO16_ID))
    );
}

#[test]
fn native_file_and_live_sources_declare_artifact_capabilities() {
    let file = node_semantics(DSL_FILE_SOURCE_ID)
        .source_data_lifecycle()
        .expect("file source lifecycle");
    assert_eq!(file.kind, SourceDataLifecycleKind::File);
    assert!(file.preload && file.cache && file.index);

    let live = node_semantics(U3PRO16_ID)
        .source_data_lifecycle()
        .expect("live source lifecycle");
    assert_eq!(live.kind, SourceDataLifecycleKind::Live);
    assert!(!live.preload && live.cache && live.index);
}

#[test]
fn buffered_hardware_feature_lowers_opaque_channels_and_portable_trigger_edits() {
    let mut widget = NodeGraphWidget::new(build_registry());
    let source = widget
        .add_node_at(node_name(U3PRO16_ID), Pos2::ZERO)
        .unwrap();
    let compiler = integration_tests_support::test_platform_compiler();
    let streaming = compiler
        .lowerer()
        .discover_live_capture_feature(widget.graph())
        .unwrap()
        .expect("stream mode should expose a live feature");
    assert_eq!(
        streaming.capabilities().data_delivery(),
        CaptureDataDelivery::DuringAcquisition
    );
    let state = &mut widget.graph_mut().nodes.get_mut(&source).unwrap().state;
    select(state, "mode", "Buffer");
    enable_channels(state, &[0, 2, 9]);

    let feature = compiler
        .lowerer()
        .discover_live_capture_feature(widget.graph())
        .unwrap()
        .expect("buffer mode should expose the concrete live feature");
    assert_eq!(feature.source_node(), source);
    assert_eq!(
        feature.channels(),
        [
            CaptureChannelId::new("u3pro16:input:0"),
            CaptureChannelId::new("u3pro16:input:2"),
            CaptureChannelId::new("u3pro16:input:9"),
        ]
    );
    assert_eq!(
        feature.capabilities().data_delivery(),
        CaptureDataDelivery::BufferedUpload
    );
    assert!(
        feature
            .capabilities()
            .supports(feature.channels(), feature.sample_rate_hz())
    );

    let edited = compiler
        .lowerer()
        .apply_live_capture_edit(
            widget.graph(),
            source,
            &LiveCaptureEdit::SetSimpleTrigger {
                channel_id: CaptureChannelId::new("u3pro16:input:2"),
                condition: SimpleTriggerCondition::Falling,
            },
        )
        .unwrap();
    widget.graph_mut().nodes.get_mut(&source).unwrap().state = edited;
    let feature = compiler
        .lowerer()
        .discover_live_capture_feature(widget.graph())
        .unwrap()
        .unwrap();
    assert_eq!(
        feature.simple_trigger_channels()[1].condition,
        SimpleTriggerCondition::Falling
    );
}

#[test]
fn buffered_hardware_discovery_rejects_too_many_channels_for_the_rate() {
    let mut widget = NodeGraphWidget::new(build_registry());
    let source = widget
        .add_node_at(node_name(U3PRO16_ID), Pos2::ZERO)
        .unwrap();
    let state = &mut widget.graph_mut().nodes.get_mut(&source).unwrap().state;
    select(state, "mode", "Buffer");
    select(state, "sample_rate", "1 GHz");
    let enabled = state["channels"]["enabled"]
        .as_array_mut()
        .expect("capture source channels are an array");
    enabled.fill(serde_json::Value::Bool(true));
    let error = GraphHarness::new()
        .lowerer()
        .discover_live_capture_feature(widget.graph())
        .err()
        .expect("wide 1 GHz buffered capture must be rejected before opening hardware");

    assert!(
        error
            .message
            .contains("Too many channels for 1 GHz in Buffer mode"),
        "{}",
        error.message
    );
}

#[test]
fn streaming_hardware_discovery_rejects_too_many_channels_for_the_rate() {
    let mut widget = NodeGraphWidget::new(build_registry());
    let source = widget
        .add_node_at(node_name(U3PRO16_ID), Pos2::ZERO)
        .unwrap();
    let state = &mut widget.graph_mut().nodes.get_mut(&source).unwrap().state;
    select(state, "mode", "Stream");
    select(state, "sample_rate", "1 GHz");
    enable_channels(state, &[0, 3]);
    let error = GraphHarness::new()
        .lowerer()
        .discover_live_capture_feature(widget.graph())
        .err()
        .expect("four-input 1 GHz stream must be rejected before opening hardware");

    assert!(
        error
            .message
            .contains("Too many channels for 1 GHz in Stream mode"),
        "{}",
        error.message
    );
}

#[test]
fn dsl_source_presentation_is_builder_owned_after_node_rename() {
    let mut widget = NodeGraphWidget::new(build_registry());
    let source_id = widget
        .add_node_at(node_name(DSL_FILE_SOURCE_ID), Pos2::ZERO)
        .expect("DSL file source should be registered");
    widget.graph_mut().nodes.get_mut(&source_id).unwrap().title = "My capture".to_owned();
    widget.graph_mut().nodes.get_mut(&source_id).unwrap().state["file"]["value"] =
        serde_json::Value::String("capture.dsl".to_owned());
    let presentation = integration_tests_support::test_platform_compiler()
        .lowerer()
        .discover_capture_presentation(widget.graph())
        .unwrap()
        .unwrap();
    let CapturePresentation::Indexed { identity, .. } = presentation.presentation else {
        panic!("DSL source should provide an indexed presentation");
    };
    assert_eq!(
        identity,
        signal_artifacts::SourceIdentity::from_bytes(*blake3::hash(b"capture.dsl").as_bytes(),)
    );
}
