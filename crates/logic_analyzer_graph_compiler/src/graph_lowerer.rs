use std::sync::Arc;

use logic_analyzer_graph_capabilities::node::{
    GraphNodeCapabilityBundle, GraphNodeCapabilityOverride,
};
use logic_analyzer_graph_capabilities::node_support::{
    LiveCaptureEdit, TimelineMarkerEdit, TimelineMarkerReferenceBindingEdit,
};
use logic_analyzer_graph_plan::{
    CapturePresentationDiscoveryError, DiscoveredCapturePresentation, OutputSubscriptionPlan,
    ProcessingGraph, ProcessingGraphError as CompileError, SamplingOverlayCandidate,
};
use logic_analyzer_graph_registry::GraphRegistry;
use node_graph_document::{GraphState, NodeId};
use signal_derived::PayloadRegistry;

use super::data_collector::{
    BUILDER_NAME as DATA_COLLECTOR_BUILDER, DataCollectorBuilder, OUTPUT_SUBSCRIPTION_BUILDER_NAME,
};
use super::error::TimelineOperationError;
use super::graph;
use super::graph::{
    DiscoveredLiveCaptureFeature, DiscoveredTimelineMarker,
    DiscoveredTimelineMarkerReferenceBinding, DiscoveredTriggerConfiguration,
    LiveCaptureDiscoveryError,
};
use super::payload_catalog::RegistryPayloadCatalog;

/// Stateless graph-document semantic analyzer and execution-plan lowerer.
///
/// The lowerer retains only the immutable graph registry and the explicit output plan.
/// It owns no artifact repository, executor, runtime manager, active run, source preparation, or
/// worker client, so invoking any method on it cannot start graph work.
pub struct GraphLowerer {
    registry: Arc<GraphRegistry>,
    output_subscriptions: OutputSubscriptionPlan,
}

impl GraphLowerer {
    /// Creates a lowerer from the validated graph registry.
    pub fn new() -> Self {
        Self::with_capability_overrides(Vec::new())
    }

    /// Creates a lowerer with composition-root graph-capability overrides.
    pub fn with_capability_overrides(
        capability_overrides: Vec<GraphNodeCapabilityOverride>,
    ) -> Self {
        Self {
            registry: Arc::new(GraphRegistry::with_capability_overrides_and_infrastructure(
                capability_overrides,
                vec![
                    (
                        DATA_COLLECTOR_BUILDER.to_owned(),
                        GraphNodeCapabilityBundle::runtime(
                            Box::new(DataCollectorBuilder::retained_data()),
                            Box::new(DataCollectorBuilder::retained_data()),
                        ),
                    ),
                    (
                        OUTPUT_SUBSCRIPTION_BUILDER_NAME.to_owned(),
                        GraphNodeCapabilityBundle::runtime(
                            Box::new(DataCollectorBuilder::output_subscription()),
                            Box::new(DataCollectorBuilder::output_subscription()),
                        ),
                    ),
                ],
            )),
            output_subscriptions: OutputSubscriptionPlan::new(),
        }
    }

    /// Replaces the explicit application-owned output-subscription plan.
    pub fn set_output_subscriptions(&mut self, subscriptions: OutputSubscriptionPlan) {
        self.output_subscriptions = subscriptions;
    }

    /// Returns the current output-subscription plan.
    pub fn output_subscriptions(&self) -> &OutputSubscriptionPlan {
        &self.output_subscriptions
    }

    /// Returns the validated registry shared with graph execution consumers.
    pub fn registry(&self) -> &GraphRegistry {
        &self.registry
    }

    /// Returns registered payload identities available during lowering.
    pub fn payloads(&self) -> &PayloadRegistry {
        self.registry.payloads()
    }

    /// Lowers a graph document without allocating or starting runtime resources.
    pub fn lower(&self, graph: &GraphState) -> Result<ProcessingGraph, Vec<CompileError>> {
        graph::lower_with_subscriptions(
            graph,
            &self.registry,
            &self.output_subscriptions,
            Arc::new(RegistryPayloadCatalog::new(Arc::clone(&self.registry))),
        )
    }

    /// Resolves sampling overlay plans without starting work.
    pub fn sampling_overlay_candidates(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<SamplingOverlayCandidate>, Vec<CompileError>> {
        graph::sampling_overlay_candidates(
            graph,
            &self.registry,
            &self.output_subscriptions,
            Arc::new(RegistryPayloadCatalog::new(Arc::clone(&self.registry))),
        )
    }

    /// Discovers the finite capture source's presentation contract.
    pub fn discover_capture_presentation(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredCapturePresentation>, CapturePresentationDiscoveryError> {
        graph::discover_capture_presentation_with_subscriptions(
            graph,
            &self.registry,
            &self.output_subscriptions,
        )
    }

    /// Discovers the graph's single live-capture feature, if present.
    pub fn discover_live_capture_feature(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredLiveCaptureFeature>, LiveCaptureDiscoveryError> {
        graph::discover_live_capture_feature_with_subscriptions(
            graph,
            &self.registry,
            &self.output_subscriptions,
        )
    }

    /// Discovers validated trigger configuration owned by a live source.
    pub fn discover_trigger_configuration(
        &self,
        graph: &GraphState,
    ) -> Result<Option<DiscoveredTriggerConfiguration>, LiveCaptureDiscoveryError> {
        graph::discover_trigger_configuration(graph, &self.registry)
    }

    /// Applies a node-owned live-capture document edit.
    pub fn apply_live_capture_edit(
        &self,
        graph: &GraphState,
        source_node: NodeId,
        edit: &LiveCaptureEdit,
    ) -> Result<serde_json::Value, String> {
        graph::apply_live_capture_edit(graph, &self.registry, source_node, edit)
    }

    /// Discovers node-owned timeline markers.
    pub fn discover_timeline_markers(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarker>, TimelineOperationError> {
        graph::discover_timeline_markers(graph, &self.registry)
    }

    /// Applies a node-owned timeline marker edit.
    pub fn apply_timeline_marker_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerEdit,
    ) -> Result<serde_json::Value, TimelineOperationError> {
        graph::apply_timeline_marker_edit(graph, &self.registry, owner_node, edit)
    }

    /// Discovers node-owned timeline-reference controls.
    pub fn discover_timeline_marker_reference_bindings(
        &self,
        graph: &GraphState,
    ) -> Result<Vec<DiscoveredTimelineMarkerReferenceBinding>, TimelineOperationError> {
        graph::discover_timeline_marker_reference_bindings(graph, &self.registry)
    }

    /// Applies node-owned timeline-reference choices.
    pub fn apply_timeline_marker_reference_binding_edit(
        &self,
        graph: &GraphState,
        owner_node: NodeId,
        edit: &TimelineMarkerReferenceBindingEdit,
    ) -> Result<serde_json::Value, TimelineOperationError> {
        graph::apply_timeline_marker_reference_binding_edit(graph, &self.registry, owner_node, edit)
    }
}

impl Default for GraphLowerer {
    fn default() -> Self {
        Self::new()
    }
}
