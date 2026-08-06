inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::EdgeDetector>("org.logicconduit.graph-node.logic.edge-detector/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::EdgeDetectorBuilder, super::builder::EdgeDetectorBuilder>(
        "org.logicconduit.graph-node.logic.edge-detector/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::EdgeDetector>,
    ).requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn edge_detector_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.edge-detector/v1",
        );
    }
}
