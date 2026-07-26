mod integration_tests_support;

use logic_analyzer_graph_api::node_support::PortKind;
use logic_analyzer_graph_compiler::{CompileCtx, GraphCompiler};
use node_graph::NodeGraphWidget;
use signal_processing::CollectedLaneSnapshotRequest;

use integration_tests_support::build_registry;

const CAMERA_PAYLOAD_ID: &str = "org.logicconduit.example.camera-frame/v1";

#[test]
fn plugin_inventory_composes_with_the_builtin_host_and_executes_typed_payloads() {
    std::hint::black_box(logic_analyzer_graph_nodes::link());
    std::hint::black_box(example_plugin::link());

    let mut compiler = GraphCompiler::new();
    assert!(
        compiler
            .payloads()
            .descriptor_by_stable_id(CAMERA_PAYLOAD_ID)
            .is_some()
    );

    let node_types = build_registry();
    assert_eq!(node_types.category_of("Pulse Measure"), Some("Plugin"));
    assert_eq!(
        node_types.category_of("Camera Frame Source"),
        Some("Plugin")
    );
    let mut widget = NodeGraphWidget::new(node_types);
    let source = widget
        .add_node_at("Camera Frame Source", egui::Pos2::ZERO)
        .expect("plugin source is registered");
    compiler.set_output_subscriptions([(source, 0)].into_iter().collect());

    let compiled = compiler.lower(widget.graph()).unwrap();
    assert!(
        compiled
            .edges
            .iter()
            .any(|edge| edge.kind == PortKind::of::<example_plugin::CameraFrame>())
    );

    let mut context = CompileCtx::default();
    let lanes = context.derived_lanes().clone();
    let mut run = compiler
        .start_app_run(widget.graph(), &mut context)
        .unwrap();
    while !run.is_finished() {
        run.pump(64);
    }

    let lane = lanes
        .opaque_lanes()
        .into_iter()
        .find(|lane| lane.payload().stable_id() == CAMERA_PAYLOAD_ID)
        .expect("plugin payload lane is published");
    assert_eq!(lane.timeline_extent_end_ns(), Some(23 * 40_000_000));
    assert!(!lane.is_live());
    assert!(
        lane.snapshot(CollectedLaneSnapshotRequest {
            start_time_ns: 0,
            end_time_ns: u64::MAX,
            max_items: 3,
        })
        .is_some()
    );
}
