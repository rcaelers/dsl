use std::hint::black_box;

use logic_analyzer_graph_capabilities::node::{GraphNodeCapabilityOverride, GraphNodePresentation};
use logic_analyzer_graph_registry::{
    GraphNodeRegistration, GraphRegistry, graph_node_registrations,
};

struct EmptyPresentation;

impl GraphNodePresentation for EmptyPresentation {}

fn linked_registrations() -> Vec<&'static GraphNodeRegistration> {
    black_box(logic_analyzer_graph_nodes::link());
    black_box(example_plugin::link());
    graph_node_registrations()
}

#[test]
fn real_graph_node_inventory_constructs_one_consistent_capability_snapshot() {
    let registrations = linked_registrations();
    assert!(
        !registrations.is_empty(),
        "real graph inventory must be linked"
    );

    let registry =
        GraphRegistry::with_capability_overrides_and_infrastructure(Vec::new(), Vec::new());
    for registration in registrations {
        let name = registration.name();
        assert_eq!(
            registry.semantics(name).is_some(),
            registration.semantics().is_some(),
            "{} semantics differ between registration and snapshot",
            registration.stable_id()
        );
        assert_eq!(
            registry.materializer(name).is_some(),
            registration.materializer().is_some(),
            "{} materialization differs between registration and snapshot",
            registration.stable_id()
        );
        assert_eq!(
            registry.capture_source(name).is_some(),
            registration.capture_source().is_some(),
            "{} capture capability differs between registration and snapshot",
            registration.stable_id()
        );
        assert_eq!(
            registry.live_capture(name).is_some(),
            registration.live_capture().is_some(),
            "{} live-capture capability differs between registration and snapshot",
            registration.stable_id()
        );
        assert_eq!(
            registry.presentation(name).is_some(),
            registration.presentation().is_some(),
            "{} presentation differs between registration and snapshot",
            registration.stable_id()
        );
        assert_eq!(
            registry.timeline(name).is_some(),
            registration.timeline().is_some(),
            "{} timeline capability differs between registration and snapshot",
            registration.stable_id()
        );
    }
}

#[test]
fn example_plugin_nodes_publish_only_semantics_and_materialization_capabilities() {
    let registrations = linked_registrations();
    for (stable_id, name) in [
        (
            "org.logicconduit.example.graph-node.camera-frame-source/v1",
            "Camera Frame Source",
        ),
        (
            "org.logicconduit.example.graph-node.pulse-measure/v1",
            "Pulse Measure",
        ),
    ] {
        let registration = registrations
            .iter()
            .find(|registration| registration.stable_id() == stable_id)
            .unwrap_or_else(|| panic!("example plugin must register {stable_id}"));

        assert_eq!(registration.name(), name);
        assert!(registration.semantics().is_some());
        assert!(registration.materializer().is_some());
        assert!(registration.capture_source().is_none());
        assert!(registration.live_capture().is_none());
        assert!(registration.presentation().is_none());
        assert!(registration.timeline().is_none());
    }
}

#[test]
fn host_capability_override_resolves_by_registered_stable_id() {
    let registrations = linked_registrations();
    let registration = registrations
        .into_iter()
        .find(|registration| {
            registration.semantics().is_some() && registration.presentation().is_none()
        })
        .expect("real inventory must contain a runnable node without presentation");

    let registry = GraphRegistry::with_capability_overrides_and_infrastructure(
        vec![
            GraphNodeCapabilityOverride::capabilities(registration.stable_id())
                .with_presentation(Box::new(EmptyPresentation)),
        ],
        Vec::new(),
    );

    assert!(registry.presentation(registration.name()).is_some());
}

#[test]
fn duplicate_host_capability_override_is_rejected() {
    let stable_id = linked_registrations()[0].stable_id();
    let result = std::panic::catch_unwind(|| {
        GraphRegistry::with_capability_overrides_and_infrastructure(
            vec![
                GraphNodeCapabilityOverride::capabilities(stable_id)
                    .with_presentation(Box::new(EmptyPresentation)),
                GraphNodeCapabilityOverride::capabilities(stable_id)
                    .with_presentation(Box::new(EmptyPresentation)),
            ],
            Vec::new(),
        )
    });

    assert!(
        result.is_err(),
        "duplicate stable-ID override must be rejected"
    );
}
