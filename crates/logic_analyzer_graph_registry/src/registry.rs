use std::collections::HashMap;
use std::sync::Arc;

use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, GraphNodeCapabilityBundle, GraphNodeCapabilityOverride,
    GraphNodePresentation, GraphNodeSemantics, LiveCaptureFeatureProvider, RuntimeMaterializer,
    TimelineFeature,
};
use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, NodeBuildContext, PortKind, ResolvedInput,
};
use signal_derived::{CollectedLaneRequest, PayloadRegistry};

use super::graph_registration::graph_node_registrations;
use super::payload_registration::{PayloadRequestConfigurator, payload_registrations};
use super::payload_request_error::PayloadRequestConfigurationError;

/// One compiler- or runtime-neutral snapshot of registered graph and payload capabilities.
pub struct GraphRegistry {
    semantics: HashMap<String, Arc<dyn GraphNodeSemantics>>,
    materializers: HashMap<String, Arc<dyn RuntimeMaterializer>>,
    capture_sources: HashMap<String, Arc<dyn CaptureSourceFeature>>,
    live_captures: HashMap<String, Arc<dyn LiveCaptureFeatureProvider>>,
    presentations: HashMap<String, Arc<dyn GraphNodePresentation>>,
    timelines: HashMap<String, Arc<dyn TimelineFeature>>,
    payloads: PayloadRegistry,
    payload_subscriptions: Vec<PayloadSubscription>,
}

struct PayloadSubscription {
    kind: PortKind,
    diagnostic_name: String,
    presentation: DefaultLanePresentationDescriptor,
    persistent_cache: bool,
    configure_request: PayloadRequestConfigurator,
}

impl GraphRegistry {
    /// Builds the validated inventory snapshot with host capability overrides and neutral
    /// infrastructure capabilities supplied by a consumer.
    pub fn with_capability_overrides_and_infrastructure(
        capability_overrides: Vec<GraphNodeCapabilityOverride>,
        infrastructure_capabilities: Vec<(String, GraphNodeCapabilityBundle)>,
    ) -> Self {
        let mut overrides = HashMap::<String, GraphNodeCapabilityBundle>::new();
        for capability_override in capability_overrides {
            let stable_id = capability_override.stable_id().to_owned();
            assert!(
                overrides
                    .insert(stable_id.clone(), capability_override.into_bundle())
                    .is_none(),
                "duplicate host graph-capability override '{stable_id}'"
            );
        }

        let mut registry = Self {
            semantics: HashMap::new(),
            materializers: HashMap::new(),
            capture_sources: HashMap::new(),
            live_captures: HashMap::new(),
            presentations: HashMap::new(),
            timelines: HashMap::new(),
            payloads: PayloadRegistry::new(),
            payload_subscriptions: Vec::new(),
        };
        for (name, mut capabilities) in infrastructure_capabilities {
            let semantics = capabilities
                .semantics
                .take()
                .expect("infrastructure graph capability requires semantics");
            let materializer = capabilities
                .materializer
                .take()
                .expect("infrastructure graph capability requires materialization");
            assert!(
                capabilities.capture_source.is_none()
                    && capabilities.live_capture.is_none()
                    && capabilities.presentation.is_none()
                    && capabilities.timeline.is_none(),
                "infrastructure graph capability '{name}' may contain only semantics and materialization"
            );
            assert!(
                registry
                    .semantics
                    .insert(name.clone(), Arc::from(semantics))
                    .is_none()
                    && registry
                        .materializers
                        .insert(name.clone(), Arc::from(materializer))
                        .is_none(),
                "duplicate infrastructure graph capability '{name}'"
            );
        }
        for registration in graph_node_registrations() {
            registration.apply_runtime_setup();
            let name = registration.name().to_owned();
            if let Some(semantics) = registration.semantics() {
                assert!(
                    registry
                        .semantics
                        .insert(name.clone(), Arc::from(semantics))
                        .is_none(),
                    "graph-node '{}' registers duplicate semantics",
                    registration.stable_id()
                );
            }
            if let Some(materializer) = registration.materializer() {
                assert!(
                    registry
                        .materializers
                        .insert(name.clone(), Arc::from(materializer))
                        .is_none(),
                    "graph-node '{}' registers duplicate materialization",
                    registration.stable_id()
                );
            }
            if let Some(capture_source) = registration.capture_source() {
                assert!(
                    registry
                        .capture_sources
                        .insert(name.clone(), Arc::from(capture_source))
                        .is_none(),
                    "graph-node '{}' registers duplicate capture-source behavior",
                    registration.stable_id()
                );
            }
            if let Some(live_capture) = registration.live_capture() {
                assert!(
                    registry
                        .live_captures
                        .insert(name.clone(), Arc::from(live_capture))
                        .is_none(),
                    "graph-node '{}' registers duplicate live-capture behavior",
                    registration.stable_id()
                );
            }
            if let Some(presentation) = registration.presentation() {
                assert!(
                    registry
                        .presentations
                        .insert(name.clone(), Arc::from(presentation))
                        .is_none(),
                    "graph-node '{}' registers duplicate presentation behavior",
                    registration.stable_id()
                );
            }
            if let Some(timeline) = registration.timeline() {
                assert!(
                    registry
                        .timelines
                        .insert(name.clone(), Arc::from(timeline))
                        .is_none(),
                    "graph-node '{}' registers duplicate timeline behavior",
                    registration.stable_id()
                );
            }
            if let Some(mut capability_override) = overrides.remove(registration.stable_id()) {
                let mut replaced = false;
                if let Some(semantics) = capability_override.semantics.take() {
                    registry
                        .semantics
                        .insert(name.clone(), Arc::from(semantics));
                    replaced = true;
                }
                if let Some(materializer) = capability_override.materializer.take() {
                    registry
                        .materializers
                        .insert(name.clone(), Arc::from(materializer));
                    replaced = true;
                }
                if let Some(capture_source) = capability_override.capture_source.take() {
                    registry
                        .capture_sources
                        .insert(name.clone(), Arc::from(capture_source));
                    replaced = true;
                }
                if let Some(live_capture) = capability_override.live_capture.take() {
                    registry
                        .live_captures
                        .insert(name.clone(), Arc::from(live_capture));
                    replaced = true;
                }
                if let Some(presentation) = capability_override.presentation.take() {
                    registry
                        .presentations
                        .insert(name.clone(), Arc::from(presentation));
                    replaced = true;
                }
                if let Some(timeline) = capability_override.timeline.take() {
                    registry.timelines.insert(name, Arc::from(timeline));
                    replaced = true;
                }
                assert!(
                    replaced,
                    "host graph-capability override '{}' contains no replacements",
                    registration.stable_id()
                );
            }
        }
        assert!(
            overrides.is_empty(),
            "host graph-capability override targets unregistered node(s): {}",
            overrides.keys().cloned().collect::<Vec<_>>().join(", ")
        );
        registry.validate_capability_combinations();
        registry.register_payloads();
        registry.validate_payload_requirements();
        registry
    }

    /// Returns registered payload identities and runtime adapters.
    pub fn payloads(&self) -> &PayloadRegistry {
        &self.payloads
    }

    /// Returns all payload kinds supporting generic output collection.
    pub fn subscribable_payload_kinds(&self) -> Vec<PortKind> {
        self.payload_subscriptions
            .iter()
            .map(|payload| payload.kind)
            .collect()
    }

    /// Returns the default presentation metadata for a subscribable payload kind.
    pub fn payload_subscription_presentation(
        &self,
        kind: PortKind,
    ) -> Option<DefaultLanePresentationDescriptor> {
        self.payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .map(|payload| payload.presentation.clone())
    }

    /// Returns whether a payload kind supports persistent indexed caching.
    pub fn payload_uses_persistent_cache(&self, kind: PortKind) -> bool {
        self.payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .is_some_and(|payload| payload.persistent_cache)
    }

    /// Applies the registered collection request customization for a payload kind.
    pub fn configure_collected_lane_request(
        &self,
        kind: PortKind,
        request: CollectedLaneRequest,
        member: usize,
        input: &ResolvedInput,
        ctx: &dyn NodeBuildContext,
    ) -> Result<(CollectedLaneRequest, &str), PayloadRequestConfigurationError> {
        let contract = self
            .payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .ok_or_else(|| PayloadRequestConfigurationError::missing_subscription(kind))?;
        Ok((
            (contract.configure_request)(request, member, input, ctx),
            &contract.diagnostic_name,
        ))
    }

    /// Returns compiler-facing graph semantics for a graph definition.
    pub fn semantics(&self, definition_name: &str) -> Option<&dyn GraphNodeSemantics> {
        self.semantics
            .get(definition_name)
            .map(|semantics| semantics.as_ref())
    }

    /// Returns the runtime-only materialization capability for a graph definition.
    pub fn materializer(&self, definition_name: &str) -> Option<Arc<dyn RuntimeMaterializer>> {
        self.materializers.get(definition_name).cloned()
    }

    /// Returns capture presentation and cache behavior for a graph definition.
    pub fn capture_source(&self, definition_name: &str) -> Option<&dyn CaptureSourceFeature> {
        self.capture_sources
            .get(definition_name)
            .map(|feature| feature.as_ref())
    }

    /// Returns live-acquisition discovery and editing behavior for a graph definition.
    pub fn live_capture(&self, definition_name: &str) -> Option<&dyn LiveCaptureFeatureProvider> {
        self.live_captures
            .get(definition_name)
            .map(|feature| feature.as_ref())
    }

    /// Returns viewer and result-presentation metadata for a graph definition.
    pub fn presentation(&self, definition_name: &str) -> Option<&dyn GraphNodePresentation> {
        self.presentations
            .get(definition_name)
            .map(|feature| feature.as_ref())
    }

    /// Returns timeline metadata and editing behavior for a graph definition.
    pub fn timeline(&self, definition_name: &str) -> Option<&dyn TimelineFeature> {
        self.timelines
            .get(definition_name)
            .map(|feature| feature.as_ref())
    }

    fn validate_capability_combinations(&self) {
        for registration in graph_node_registrations() {
            let name = registration.name();
            let semantics = self.semantics.contains_key(name);
            let materializer = self.materializers.contains_key(name);
            assert!(
                semantics == materializer,
                "graph-node '{}' must register semantics and materialization together",
                registration.stable_id()
            );

            let capture_source = self.capture_sources.contains_key(name);
            let live_capture = self.live_captures.contains_key(name);
            let presentation = self.presentations.contains_key(name);
            assert!(
                !capture_source
                    || self
                        .semantics
                        .get(name)
                        .is_some_and(|semantics| semantics.is_source()),
                "graph-node '{}' registers capture behavior without source semantics",
                registration.stable_id()
            );
            assert!(
                !live_capture || capture_source,
                "graph-node '{}' registers live capture without capture-source behavior",
                registration.stable_id()
            );
            assert!(
                !live_capture || presentation,
                "graph-node '{}' registers live capture without channel presentation",
                registration.stable_id()
            );
        }
    }

    fn register_payloads(&mut self) {
        for registration in payload_registrations() {
            let kind = registration.kind();
            kind.register_runtime_type();
            self.payloads
                .register_erased(kind.type_id(), registration.stable_id())
                .expect("payload inventory stable IDs and types are validated");
            self.payloads
                .register_adapter_erased(kind.type_id(), kind.name(), registration.adapter())
                .expect("payload inventory adapters are unique");
            self.payload_subscriptions.push(PayloadSubscription {
                kind,
                diagnostic_name: kind.name().to_owned(),
                presentation: registration.presentation(),
                persistent_cache: registration.persistent_cache(),
                configure_request: registration.configure_request(),
            });
        }
    }

    fn validate_payload_requirements(&self) {
        for registration in graph_node_registrations() {
            for stable_id in registration.required_payloads() {
                assert!(
                    self.payloads.descriptor_by_stable_id(stable_id).is_some(),
                    "graph-node inventory feature '{}' requires unavailable payload '{}'",
                    registration.stable_id(),
                    stable_id
                );
            }
        }
    }
}
