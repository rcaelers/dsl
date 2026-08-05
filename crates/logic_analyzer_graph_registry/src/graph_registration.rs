use std::collections::HashSet;

use logic_analyzer_graph_capabilities::node::{
    CaptureSourceFeature, GraphNodePresentation, GraphNodeSemantics, LiveCaptureFeatureProvider,
    RuntimeMaterializer, TimelineFeature,
};
use node_graph::api::{NodeDef, NodeTypeRegistry};

/// Inventory submission describing one graph-node feature.
///
/// `stable_id` is the persisted feature identity; it is intentionally separate from the editable
/// display name supplied by [`NodeDef`].
pub struct GraphNodeRegistration {
    stable_id: &'static str,
    node_name: fn() -> &'static str,
    register_node: fn(&mut NodeTypeRegistry),
    create_semantics: Option<fn() -> Box<dyn GraphNodeSemantics>>,
    create_materializer: Option<fn() -> Box<dyn RuntimeMaterializer>>,
    create_capture_source: Option<fn() -> Box<dyn CaptureSourceFeature>>,
    create_live_capture: Option<fn() -> Box<dyn LiveCaptureFeatureProvider>>,
    create_presentation: Option<fn() -> Box<dyn GraphNodePresentation>>,
    create_timeline: Option<fn() -> Box<dyn TimelineFeature>>,
    required_payloads: &'static [&'static str],
    runtime_setup: &'static [fn()],
}

impl GraphNodeRegistration {
    /// Registers a node definition with separate semantic and materialization capabilities.
    pub const fn capable<N, S, M>(stable_id: &'static str) -> Self
    where
        N: NodeDef,
        S: GraphNodeSemantics + Default + 'static,
        M: RuntimeMaterializer + Default + 'static,
    {
        Self {
            stable_id,
            node_name: node_name::<N>,
            register_node: register_node::<N>,
            create_semantics: Some(create_semantics::<S>),
            create_materializer: Some(create_materializer::<M>),
            create_capture_source: None,
            create_live_capture: None,
            create_presentation: None,
            create_timeline: None,
            required_payloads: &[],
            runtime_setup: &[],
        }
    }

    /// Registers a definition-only node that cannot materialize a runtime node.
    pub const fn definition<N: NodeDef>(stable_id: &'static str) -> Self {
        Self {
            stable_id,
            node_name: node_name::<N>,
            register_node: register_node::<N>,
            create_semantics: None,
            create_materializer: None,
            create_capture_source: None,
            create_live_capture: None,
            create_presentation: None,
            create_timeline: None,
            required_payloads: &[],
            runtime_setup: &[],
        }
    }

    /// Declares payload stable IDs required before this feature can be used.
    pub const fn requiring_payloads(mut self, required_payloads: &'static [&'static str]) -> Self {
        self.required_payloads = required_payloads;
        self
    }

    /// Adds setup functions that run before runtime capabilities are created.
    pub const fn with_runtime_setup(mut self, runtime_setup: &'static [fn()]) -> Self {
        self.runtime_setup = runtime_setup;
        self
    }

    /// Adds capture presentation and cache behavior to this registration.
    pub const fn with_capture_source<C>(mut self) -> Self
    where
        C: CaptureSourceFeature + Default + 'static,
    {
        self.create_capture_source = Some(create_capture_source::<C>);
        self
    }

    /// Adds live-acquisition discovery and editing behavior to this registration.
    pub const fn with_live_capture<L>(mut self) -> Self
    where
        L: LiveCaptureFeatureProvider + Default + 'static,
    {
        self.create_live_capture = Some(create_live_capture::<L>);
        self
    }

    /// Adds viewer and result-presentation metadata to this registration.
    pub const fn with_presentation<P>(mut self) -> Self
    where
        P: GraphNodePresentation + Default + 'static,
    {
        self.create_presentation = Some(create_presentation::<P>);
        self
    }

    /// Adds timeline metadata and editing behavior to this registration.
    pub const fn with_timeline<T>(mut self) -> Self
    where
        T: TimelineFeature + Default + 'static,
    {
        self.create_timeline = Some(create_timeline::<T>);
        self
    }

    /// Returns the stable persisted feature identifier.
    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    /// Returns the node definition's display name.
    pub fn name(&self) -> &'static str {
        (self.node_name)()
    }

    /// Returns the required payload stable IDs.
    pub const fn required_payloads(&self) -> &'static [&'static str] {
        self.required_payloads
    }

    /// Runs this feature's registered runtime setup hooks.
    pub fn apply_runtime_setup(&self) {
        for setup in self.runtime_setup {
            setup();
        }
    }

    /// Registers the feature's node definition with an editor registry.
    pub fn apply_node(&self, registry: &mut NodeTypeRegistry) {
        (self.register_node)(registry);
    }

    /// Creates the feature's explicit graph semantics, when registered separately.
    pub fn semantics(&self) -> Option<Box<dyn GraphNodeSemantics>> {
        self.create_semantics
            .map(|create_semantics| create_semantics())
    }

    /// Creates the feature's explicit runtime materializer, when registered separately.
    pub fn materializer(&self) -> Option<Box<dyn RuntimeMaterializer>> {
        self.create_materializer
            .map(|create_materializer| create_materializer())
    }

    /// Creates the feature's capture-source capability, when registered.
    pub fn capture_source(&self) -> Option<Box<dyn CaptureSourceFeature>> {
        self.create_capture_source
            .map(|create_capture_source| create_capture_source())
    }

    /// Creates the feature's live-capture capability, when registered.
    pub fn live_capture(&self) -> Option<Box<dyn LiveCaptureFeatureProvider>> {
        self.create_live_capture
            .map(|create_live_capture| create_live_capture())
    }

    /// Creates the feature's presentation capability, when registered.
    pub fn presentation(&self) -> Option<Box<dyn GraphNodePresentation>> {
        self.create_presentation
            .map(|create_presentation| create_presentation())
    }

    /// Creates the feature's timeline capability, when registered.
    pub fn timeline(&self) -> Option<Box<dyn TimelineFeature>> {
        self.create_timeline
            .map(|create_timeline| create_timeline())
    }
}

fn node_name<N: NodeDef>() -> &'static str {
    N::name()
}

fn register_node<N: NodeDef>(registry: &mut NodeTypeRegistry) {
    registry.register::<N>();
}

fn create_semantics<S: GraphNodeSemantics + Default + 'static>() -> Box<dyn GraphNodeSemantics> {
    Box::<S>::default()
}

fn create_materializer<M: RuntimeMaterializer + Default + 'static>() -> Box<dyn RuntimeMaterializer>
{
    Box::<M>::default()
}

fn create_capture_source<C: CaptureSourceFeature + Default + 'static>()
-> Box<dyn CaptureSourceFeature> {
    Box::<C>::default()
}

fn create_live_capture<L: LiveCaptureFeatureProvider + Default + 'static>()
-> Box<dyn LiveCaptureFeatureProvider> {
    Box::<L>::default()
}

fn create_presentation<P: GraphNodePresentation + Default + 'static>()
-> Box<dyn GraphNodePresentation> {
    Box::<P>::default()
}

fn create_timeline<T: TimelineFeature + Default + 'static>() -> Box<dyn TimelineFeature> {
    Box::<T>::default()
}

inventory::collect!(GraphNodeRegistration);

/// Returns validated graph-node registrations in stable-ID order.
pub fn graph_node_registrations() -> Vec<&'static GraphNodeRegistration> {
    let mut registrations = inventory::iter::<GraphNodeRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    validate_graph_node_registrations(&mut registrations);
    registrations
}

fn validate_graph_node_registrations(registrations: &mut Vec<&GraphNodeRegistration>) {
    registrations.sort_by_key(|registration| registration.stable_id());
    let mut stable_ids = HashSet::new();
    let mut names = HashSet::new();
    for registration in registrations {
        assert!(
            !registration.stable_id().trim().is_empty(),
            "graph-node inventory contains an empty stable ID"
        );
        assert!(
            stable_ids.insert(registration.stable_id()),
            "duplicate graph-node inventory stable ID '{}'",
            registration.stable_id()
        );
        assert!(
            names.insert(registration.name()),
            "duplicate graph-node inventory name '{}'",
            registration.name()
        );
        assert!(
            registration.create_semantics.is_some() == registration.create_materializer.is_some(),
            "graph-node '{}' must register semantics and materialization together",
            registration.stable_id()
        );
        assert!(
            registration.create_live_capture.is_none()
                || registration.create_capture_source.is_some(),
            "graph-node '{}' registers live capture without capture-source behavior",
            registration.stable_id()
        );
        assert!(
            registration.create_live_capture.is_none()
                || registration.create_presentation.is_some(),
            "graph-node '{}' registers live capture without channel presentation",
            registration.stable_id()
        );
    }
}

#[cfg(test)]
mod graph_registration_tests {
    use super::*;

    struct FirstNode;

    impl NodeDef for FirstNode {
        type State = ();

        fn name() -> &'static str {
            "First"
        }

        fn category() -> &'static str {
            "Tests"
        }

        fn inputs() -> Vec<node_graph::api::InputDef<Self::State>> {
            Vec::new()
        }

        fn outputs() -> Vec<node_graph::api::OutputDef<Self::State>> {
            Vec::new()
        }

        fn state() -> Self::State {}
    }

    struct SecondNode;

    impl NodeDef for SecondNode {
        type State = ();

        fn name() -> &'static str {
            "Second"
        }

        fn category() -> &'static str {
            "Tests"
        }

        fn inputs() -> Vec<node_graph::api::InputDef<Self::State>> {
            Vec::new()
        }

        fn outputs() -> Vec<node_graph::api::OutputDef<Self::State>> {
            Vec::new()
        }

        fn state() -> Self::State {}
    }

    #[test]
    fn validation_sorts_registrations_by_stable_id() {
        let first = GraphNodeRegistration::definition::<FirstNode>("a");
        let second = GraphNodeRegistration::definition::<SecondNode>("b");
        let mut registrations = vec![&second, &first];

        validate_graph_node_registrations(&mut registrations);

        assert_eq!(registrations[0].stable_id(), "a");
        assert_eq!(registrations[1].stable_id(), "b");
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let registration = GraphNodeRegistration::definition::<FirstNode>("duplicate");
        let mut registrations = vec![&registration, &registration];
        assert!(
            std::panic::catch_unwind(move || {
                validate_graph_node_registrations(&mut registrations)
            })
            .is_err()
        );
    }
}
