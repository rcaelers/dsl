inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<
        super::definition::SigrokFileSource,
        super::builder::SigrokFileSourceBuilder,
        super::builder::SigrokFileSourceBuilder,
    >("org.logicconduit.graph-node.sources.sigrok-file-source/v1")
    .with_capture_source::<super::builder::SigrokFileSourceBuilder>()
    .with_presentation::<super::builder::SigrokFileSourceBuilder>()
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    use node_graph::NodeDef;

    #[test]
    fn sigrok_file_source_registration_contract_accepts_demo_state() {
        let mut state = super::super::definition::SigrokFileSource::state();
        state.demo_data = true;
        crate::nodes::test_support::assert_node_registration_contract_with_state(
            "org.logicconduit.graph-node.sources.sigrok-file-source/v1",
            Some(serde_json::to_value(state).expect("test state is serializable")),
        );
    }
}
