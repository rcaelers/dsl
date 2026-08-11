use logic_analyzer_graph_capabilities::node::{
    GraphNodePresentation, GraphNodeSemantics, RuntimeMaterializationError, RuntimeMaterializer,
};
use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, LaneBadgeDescriptor, NodeBuildContext, PortKind,
    ResolvedInputs,
};
use logic_analyzer_graph_registry::{GraphNodeRegistration, PayloadRegistration};
use node_graph::api::{AnySocket, InputDef, NodeDef, NodeTypeRegistry, OutputDef};
use node_graph_document::SocketReference;
use signal_derived::Word;
use signal_runtime::ProcessNode;

pub(crate) const WORD_PRODUCER_NAME: &str = "UI Test Word Producer";

struct WordProducerDefinition;

impl NodeDef for WordProducerDefinition {
    type State = serde_json::Value;

    fn name() -> &'static str {
        WORD_PRODUCER_NAME
    }

    fn category() -> &'static str {
        "UI Tests"
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        Vec::new()
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<AnySocket>("Words")]
    }

    fn state() -> Self::State {
        serde_json::Value::Null
    }
}

#[derive(Default)]
struct WordProducerBuilder;

impl GraphNodeSemantics for WordProducerBuilder {
    fn accepted_kinds(
        &self,
        _socket: SocketReference<'_>,
        _state: &serde_json::Value,
    ) -> Vec<PortKind> {
        Vec::new()
    }

    fn offered_kinds(
        &self,
        _socket: SocketReference<'_>,
        _state: &serde_json::Value,
    ) -> Vec<PortKind> {
        vec![PortKind::of::<Word>()]
    }

    fn input_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &serde_json::Value,
        _kind: PortKind,
    ) -> Option<String> {
        None
    }

    fn output_port(
        &self,
        _socket: SocketReference<'_>,
        _state: &serde_json::Value,
        kind: PortKind,
    ) -> Option<String> {
        (kind == PortKind::of::<Word>()).then(|| "words".to_owned())
    }
}

impl RuntimeMaterializer for WordProducerBuilder {
    fn build(
        &self,
        _name: &str,
        _state: &serde_json::Value,
        _resolved: &ResolvedInputs,
        _context: &mut dyn NodeBuildContext,
    ) -> Result<Box<dyn ProcessNode>, RuntimeMaterializationError> {
        Err(RuntimeMaterializationError::contract(
            "UI test producer is graph-only",
        ))
    }
}

impl GraphNodePresentation for WordProducerBuilder {}

struct LegacyViewerDefinition;

impl NodeDef for LegacyViewerDefinition {
    type State = serde_json::Value;

    fn name() -> &'static str {
        "Viewer"
    }

    fn category() -> &'static str {
        "UI Tests"
    }

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<AnySocket>("In").variadic(16)]
    }

    fn outputs() -> Vec<OutputDef<Self::State>> {
        Vec::new()
    }

    fn state() -> Self::State {
        serde_json::Value::Null
    }
}

fn word_presentation() -> DefaultLanePresentationDescriptor {
    DefaultLanePresentationDescriptor::new(
        LaneBadgeDescriptor::new("W", [215, 140, 60]),
        "org.logicconduit.ui-test.word-renderer/v1",
    )
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<WordProducerDefinition>(
        "org.logicconduit.ui-test.word-producer/v1",
    )
}

inventory::submit! {
    GraphNodeRegistration::capable::<WordProducerBuilder, WordProducerBuilder>(
        "org.logicconduit.ui-test.word-producer/v1",
        logic_analyzer_graph_editor_registry::node_name::<WordProducerDefinition>,
    )
    .with_presentation::<WordProducerBuilder>()
}

inventory::submit! {
    logic_analyzer_graph_editor_registry::GraphNodeEditorRegistration::definition::<LegacyViewerDefinition>(
        crate::viewer_selection::LEGACY_VIEWER_NODE_ID,
    )
}

inventory::submit! {
    GraphNodeRegistration::definition(
        crate::viewer_selection::LEGACY_VIEWER_NODE_ID,
        logic_analyzer_graph_editor_registry::node_name::<LegacyViewerDefinition>,
    )
}

inventory::submit! {
    PayloadRegistration::subscribable::<Word>(
        "org.logicconduit.word/v1",
        signal_derived::word_payload_adapter,
        word_presentation,
    )
}

pub(crate) fn build_test_node_registry() -> NodeTypeRegistry {
    let mut registry = NodeTypeRegistry::new();
    for registration in logic_analyzer_graph_editor_registry::graph_node_editor_registrations() {
        registration.apply_node(&mut registry);
    }
    registry
}
