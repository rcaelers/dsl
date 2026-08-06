inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::CursorMarker>("org.logicconduit.graph-node.timeline.cursor-marker/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::CursorMarkerBuilder, super::builder::CursorMarkerBuilder>(
        "org.logicconduit.graph-node.timeline.cursor-marker/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::CursorMarker>,
    )
    .with_timeline::<super::builder::CursorMarkerBuilder>()
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::TimelineMarker>("org.logicconduit.graph-node.timeline.marker/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::TimelineMarkerBuilder, super::builder::TimelineMarkerBuilder>(
        "org.logicconduit.graph-node.timeline.marker/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::TimelineMarker>,
    )
    .with_timeline::<super::builder::TimelineMarkerBuilder>()
    .with_runtime_setup(&[super::builder::register_timeline_marker_type])
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::MarkerToTrigger>("org.logicconduit.graph-node.timeline.marker-to-trigger/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::MarkerToTriggerBuilder, super::builder::MarkerToTriggerBuilder>(
        "org.logicconduit.graph-node.timeline.marker-to-trigger/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::MarkerToTrigger>,
    )
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::MarkerRelation>("org.logicconduit.graph-node.timeline.marker-relation/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::MarkerRelationBuilder, super::builder::MarkerRelationBuilder>(
        "org.logicconduit.graph-node.timeline.marker-relation/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::MarkerRelation>,
    )
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::MarkerWindow>("org.logicconduit.graph-node.timeline.marker-window/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::MarkerWindowBuilder, super::builder::MarkerWindowBuilder>(
        "org.logicconduit.graph-node.timeline.marker-window/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::MarkerWindow>,
    )
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
