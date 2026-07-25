use std::collections::HashSet;

use node_graph::api::{NodeDef, NodeTypeRegistry};

use super::contracts::RuntimeBuilder;

pub struct GraphNodeRegistration {
    stable_id: &'static str,
    node_name: fn() -> &'static str,
    register_node: fn(&mut NodeTypeRegistry),
    create_builder: Option<fn() -> Box<dyn RuntimeBuilder>>,
    required_payloads: &'static [&'static str],
    runtime_setup: &'static [fn()],
}

impl GraphNodeRegistration {
    pub const fn runnable<N, B>(stable_id: &'static str) -> Self
    where
        N: NodeDef,
        B: RuntimeBuilder + Default + 'static,
    {
        Self {
            stable_id,
            node_name: node_name::<N>,
            register_node: register_node::<N>,
            create_builder: Some(create_builder::<B>),
            required_payloads: &[],
            runtime_setup: &[],
        }
    }

    pub const fn definition<N: NodeDef>(stable_id: &'static str) -> Self {
        Self {
            stable_id,
            node_name: node_name::<N>,
            register_node: register_node::<N>,
            create_builder: None,
            required_payloads: &[],
            runtime_setup: &[],
        }
    }

    pub const fn requiring_payloads(mut self, required_payloads: &'static [&'static str]) -> Self {
        self.required_payloads = required_payloads;
        self
    }

    pub const fn with_runtime_setup(mut self, runtime_setup: &'static [fn()]) -> Self {
        self.runtime_setup = runtime_setup;
        self
    }

    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    pub fn name(&self) -> &'static str {
        (self.node_name)()
    }

    pub const fn required_payloads(&self) -> &'static [&'static str] {
        self.required_payloads
    }

    #[doc(hidden)]
    pub fn apply_runtime_setup(&self) {
        for setup in self.runtime_setup {
            setup();
        }
    }

    #[doc(hidden)]
    pub fn apply_node(&self, registry: &mut NodeTypeRegistry) {
        (self.register_node)(registry);
    }

    #[doc(hidden)]
    pub fn builder(&self) -> Option<Box<dyn RuntimeBuilder>> {
        self.create_builder.map(|create_builder| create_builder())
    }
}

fn node_name<N: NodeDef>() -> &'static str {
    N::name()
}

fn register_node<N: NodeDef>(registry: &mut NodeTypeRegistry) {
    registry.register::<N>();
}

fn create_builder<B: RuntimeBuilder + Default + 'static>() -> Box<dyn RuntimeBuilder> {
    Box::<B>::default()
}

inventory::collect!(GraphNodeRegistration);

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
    }
}

#[cfg(test)]
mod graph_registration_tests {
    use super::*;

    struct FirstNode;

    impl node_graph::api::NodeDef for FirstNode {
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

    impl node_graph::api::NodeDef for SecondNode {
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
