inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::SrFlipFlop>("org.logicconduit.graph-node.logic.sr-flip-flop/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::SrFlipFlopBuilder, super::builder::SrFlipFlopBuilder>(
        "org.logicconduit.graph-node.logic.sr-flip-flop/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::SrFlipFlop>,
    ).requiring_payloads(&[
        "org.logicconduit.digital-sample/v1",
        "org.logicconduit.trigger/v1",
    ])
}

#[cfg(test)]
mod registration_tests {
    #[test]
    fn sr_flip_flop_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(
            "org.logicconduit.graph-node.logic.sr-flip-flop/v1",
        );
    }
}
