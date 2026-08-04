inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::CursorMarker,
        super::builder::CursorMarkerBuilder,
    >("org.logicconduit.graph-node.timeline.cursor-marker/v1")
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::TimelineMarker,
        super::builder::TimelineMarkerBuilder,
    >("org.logicconduit.graph-node.timeline.marker/v1")
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::MarkerToTrigger,
        super::builder::MarkerToTriggerBuilder,
    >("org.logicconduit.graph-node.timeline.marker-to-trigger/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::MarkerRelation,
        super::builder::MarkerRelationBuilder,
    >("org.logicconduit.graph-node.timeline.marker-relation/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::runnable::<
        super::definition::MarkerWindow,
        super::builder::MarkerWindowBuilder,
    >("org.logicconduit.graph-node.timeline.marker-window/v1")
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn timeline_marker_registrations_are_self_consistent() {
        for stable_id in [
            "org.logicconduit.graph-node.timeline.marker/v1",
            "org.logicconduit.graph-node.timeline.marker-to-trigger/v1",
            "org.logicconduit.graph-node.timeline.marker-relation/v1",
            "org.logicconduit.graph-node.timeline.marker-window/v1",
        ] {
            crate::nodes::test_support::assert_node_registration_contract(stable_id);
        }
        crate::nodes::test_support::assert_node_registration_contract_without_runtime(
            "org.logicconduit.graph-node.timeline.cursor-marker/v1",
        );
    }
}
