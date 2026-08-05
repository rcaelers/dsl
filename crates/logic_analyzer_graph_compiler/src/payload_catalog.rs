use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{NodeBuildContext, PortKind, ResolvedInput};
use logic_analyzer_graph_plan::ProcessingPayloadCatalog;
use logic_analyzer_graph_registry::GraphRegistry;
use signal_derived::{CollectedLaneRequest, PayloadRegistry};

pub(crate) struct RegistryPayloadCatalog {
    registry: Arc<GraphRegistry>,
}

impl RegistryPayloadCatalog {
    pub(crate) fn new(registry: Arc<GraphRegistry>) -> Self {
        Self { registry }
    }
}

impl ProcessingPayloadCatalog for RegistryPayloadCatalog {
    fn payloads(&self) -> &PayloadRegistry {
        self.registry.payloads()
    }

    fn uses_persistent_cache(&self, kind: PortKind) -> bool {
        self.registry.payload_uses_persistent_cache(kind)
    }

    fn configure_collected_lane_request(
        &self,
        kind: PortKind,
        request: CollectedLaneRequest,
        member: usize,
        input: &ResolvedInput,
        context: &dyn NodeBuildContext,
    ) -> Result<(CollectedLaneRequest, &str), String> {
        self.registry
            .configure_collected_lane_request(kind, request, member, input, context)
    }
}
