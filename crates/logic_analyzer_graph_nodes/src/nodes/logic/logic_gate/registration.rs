inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<super::definition::LogicGate>("org.logicconduit.graph-node.logic.logic-gate/v1")
}

inventory::submit! {
    logic_analyzer_graph_registry::GraphNodeRegistration::capable::<super::builder::LogicGateBuilder, super::builder::LogicGateBuilder>(
        "org.logicconduit.graph-node.logic.logic-gate/v1",
        logic_analyzer_graph_editor_registry::node_name::<super::definition::LogicGate>,
    )
    .requiring_payloads(&["org.logicconduit.digital-sample/v1"])
}

#[cfg(test)]
mod registration_tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use logic_analyzer_graph_capabilities::node::{
        GraphNodeCapabilityOverride, GraphNodePresentation, LiveCaptureFeature,
        LiveCaptureFeatureProvider,
    };
    use logic_analyzer_graph_registry::{GraphRegistry, graph_node_registrations};

    const STABLE_ID: &str = "org.logicconduit.graph-node.logic.logic-gate/v1";

    struct EmptyPresentation;

    impl GraphNodePresentation for EmptyPresentation {}

    struct InvalidLiveCapture;

    impl LiveCaptureFeatureProvider for InvalidLiveCapture {
        fn live_capture_feature(
            &self,
            _state: &serde_json::Value,
        ) -> Result<Option<Box<dyn LiveCaptureFeature>>, String> {
            Ok(None)
        }
    }

    #[test]
    fn logic_gate_registration_contract_is_self_consistent() {
        crate::nodes::test_support::assert_node_registration_contract(STABLE_ID);
    }

    #[test]
    fn logic_gate_registration_exposes_only_narrow_runtime_capabilities() {
        let registration = graph_node_registrations()
            .into_iter()
            .find(|registration| registration.stable_id() == STABLE_ID)
            .expect("logic-gate registration");

        assert!(registration.semantics().is_some());
        assert!(registration.materializer().is_some());
    }

    #[test]
    fn host_can_replace_one_narrow_capability_without_installing_a_builder() {
        let registry = GraphRegistry::with_capability_overrides_and_infrastructure(
            vec![
                GraphNodeCapabilityOverride::capabilities(STABLE_ID)
                    .with_presentation(Box::new(EmptyPresentation)),
            ],
            Vec::new(),
        );

        assert!(registry.presentation("Logic Gate").is_some());
        assert!(registry.semantics("Logic Gate").is_some());
        assert!(registry.materializer("Logic Gate").is_some());
    }

    #[test]
    fn registry_rejects_empty_duplicate_and_invalid_capability_overrides() {
        let empty = catch_unwind(AssertUnwindSafe(|| {
            GraphRegistry::with_capability_overrides_and_infrastructure(
                vec![GraphNodeCapabilityOverride::capabilities(STABLE_ID)],
                Vec::new(),
            )
        }));
        assert!(empty.is_err());

        let duplicate = catch_unwind(AssertUnwindSafe(|| {
            GraphRegistry::with_capability_overrides_and_infrastructure(
                vec![
                    GraphNodeCapabilityOverride::capabilities(STABLE_ID)
                        .with_presentation(Box::new(EmptyPresentation)),
                    GraphNodeCapabilityOverride::capabilities(STABLE_ID)
                        .with_presentation(Box::new(EmptyPresentation)),
                ],
                Vec::new(),
            )
        }));
        assert!(duplicate.is_err());

        let invalid_combination = catch_unwind(AssertUnwindSafe(|| {
            GraphRegistry::with_capability_overrides_and_infrastructure(
                vec![
                    GraphNodeCapabilityOverride::capabilities(STABLE_ID)
                        .with_live_capture(Box::new(InvalidLiveCapture)),
                ],
                Vec::new(),
            )
        }));
        assert!(invalid_combination.is_err());
    }
}
