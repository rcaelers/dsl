use std::collections::HashMap;
use std::sync::Arc;

use logic_analyzer_graph_capabilities::node::{RuntimeBuilder, RuntimeBuilderOverride};
use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, NodeBuildContext, PortKind, ResolvedInput,
};
use signal_processing::{CollectedLaneRequest, PayloadRegistry};

use super::graph_registration::graph_node_registrations;
use super::payload_registration::{PayloadRequestConfigurator, payload_registrations};

/// One compiler- or runtime-neutral snapshot of registered graph and payload capabilities.
pub struct GraphRegistry {
    builders: HashMap<String, Arc<dyn RuntimeBuilder>>,
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
    /// Builds the validated inventory snapshot with host overrides and neutral infrastructure
    /// builders supplied by a consumer.
    pub fn with_builder_overrides_and_infrastructure(
        overrides: Vec<RuntimeBuilderOverride>,
        infrastructure_builders: Vec<(String, Arc<dyn RuntimeBuilder>)>,
    ) -> Self {
        let mut builders = HashMap::new();
        for (name, builder) in infrastructure_builders {
            assert!(
                builders.insert(name.clone(), builder).is_none(),
                "duplicate infrastructure graph builder '{name}'"
            );
        }
        let mut overrides = overrides
            .into_iter()
            .map(|override_builder| {
                let stable_id = override_builder.stable_id().to_owned();
                (stable_id, Arc::from(override_builder.into_builder()))
            })
            .collect::<HashMap<_, _>>();

        for registration in graph_node_registrations() {
            registration.apply_runtime_setup();
            let builder = overrides
                .remove(registration.stable_id())
                .or_else(|| registration.builder().map(Arc::from));
            let Some(builder) = builder else {
                continue;
            };
            assert!(
                builders
                    .insert(registration.name().to_owned(), builder)
                    .is_none(),
                "graph-node inventory builder '{}' conflicts with an explicit catalog entry",
                registration.name()
            );
        }
        assert!(
            overrides.is_empty(),
            "host runtime-builder override targets unregistered node(s): {}",
            overrides.keys().cloned().collect::<Vec<_>>().join(", ")
        );

        let mut registry = Self {
            builders,
            payloads: PayloadRegistry::new(),
            payload_subscriptions: Vec::new(),
        };
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
    ) -> Result<(CollectedLaneRequest, &str), String> {
        let contract = self
            .payload_subscriptions
            .iter()
            .find(|payload| payload.kind == kind)
            .ok_or_else(|| format!("payload {kind:?} has no data-subscription contract"))?;
        Ok((
            (contract.configure_request)(request, member, input, ctx),
            &contract.diagnostic_name,
        ))
    }

    /// Returns the runtime builder registered for a graph definition name.
    pub fn get(&self, definition_name: &str) -> Option<&dyn RuntimeBuilder> {
        self.builders
            .get(definition_name)
            .map(|builder| builder.as_ref())
    }

    /// Returns shared ownership of the runtime builder for a graph definition name.
    pub fn builder(&self, definition_name: &str) -> Option<Arc<dyn RuntimeBuilder>> {
        self.builders.get(definition_name).cloned()
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
