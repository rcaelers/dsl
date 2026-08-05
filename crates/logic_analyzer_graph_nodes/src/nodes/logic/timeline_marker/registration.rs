inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::CursorMarker,
        super::builder::CursorMarkerBuilder,
        super::builder::CursorMarkerBuilder,
    >("org.logicconduit.graph-node.timeline.cursor-marker/v1")
    .with_timeline::<super::builder::CursorMarkerBuilder>()
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::TimelineMarker,
        super::builder::TimelineMarkerBuilder,
        super::builder::TimelineMarkerBuilder,
    >("org.logicconduit.graph-node.timeline.marker/v1")
    .with_timeline::<super::builder::TimelineMarkerBuilder>()
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::MarkerToTrigger,
        super::builder::MarkerToTriggerBuilder,
        super::builder::MarkerToTriggerBuilder,
    >("org.logicconduit.graph-node.timeline.marker-to-trigger/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::MarkerRelation,
        super::builder::MarkerRelationBuilder,
        super::builder::MarkerRelationBuilder,
    >("org.logicconduit.graph-node.timeline.marker-relation/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::MarkerWindow,
        super::builder::MarkerWindowBuilder,
        super::builder::MarkerWindowBuilder,
    >("org.logicconduit.graph-node.timeline.marker-window/v1")
}

#[cfg(test)]
mod registration_tests {
    use logic_analyzer_graph_registry::graph_node_registrations;

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

    #[test]
    fn marker_and_cursor_register_explicit_timeline_features() {
        for stable_id in [
            "org.logicconduit.graph-node.timeline.marker/v1",
            "org.logicconduit.graph-node.timeline.cursor-marker/v1",
        ] {
            let registration = graph_node_registrations()
                .into_iter()
                .find(|registration| registration.stable_id() == stable_id)
                .unwrap_or_else(|| panic!("missing registration '{stable_id}'"));
            assert!(registration.timeline().is_some());
        }
    }
}
