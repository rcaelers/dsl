use std::collections::HashSet;

use node_graph::api::{NodeDef, NodeTypeRegistry};

/// Inventory submission for one graph feature's node-editor definition.
pub struct GraphNodeEditorRegistration {
    stable_id: &'static str,
    node_name: fn() -> &'static str,
    register_node: fn(&mut NodeTypeRegistry),
}

impl GraphNodeEditorRegistration {
    /// Registers one editor definition under its persisted feature identity.
    pub const fn definition<N: NodeDef>(stable_id: &'static str) -> Self {
        Self {
            stable_id,
            node_name: node_name::<N>,
            register_node: register_node::<N>,
        }
    }

    /// Returns the stable persisted feature identifier.
    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    /// Returns the node definition's display name.
    pub fn name(&self) -> &'static str {
        (self.node_name)()
    }

    /// Registers the definition with a node editor registry.
    pub fn apply_node(&self, registry: &mut NodeTypeRegistry) {
        (self.register_node)(registry);
    }
}

/// Returns the display name of a node-editor definition.
///
/// Headless feature registrations accept this function as an injected identity accessor without
/// depending on the node editor or its `NodeDef` contract.
pub fn node_name<N: NodeDef>() -> &'static str {
    N::name()
}

fn register_node<N: NodeDef>(registry: &mut NodeTypeRegistry) {
    registry.register::<N>();
}

inventory::collect!(GraphNodeEditorRegistration);

/// Returns validated editor registrations in stable-ID order.
pub fn graph_node_editor_registrations() -> Vec<&'static GraphNodeEditorRegistration> {
    let mut registrations = inventory::iter::<GraphNodeEditorRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    validate_graph_node_editor_registrations(&mut registrations);
    registrations
}

fn validate_graph_node_editor_registrations(registrations: &mut Vec<&GraphNodeEditorRegistration>) {
    registrations.sort_by_key(|registration| registration.stable_id());
    let mut stable_ids = HashSet::new();
    let mut names = HashSet::new();
    for registration in registrations {
        assert!(
            !registration.stable_id().trim().is_empty(),
            "graph-node editor inventory contains an empty stable ID"
        );
        assert!(
            stable_ids.insert(registration.stable_id()),
            "duplicate graph-node editor inventory stable ID '{}'",
            registration.stable_id()
        );
        assert!(
            names.insert(registration.name()),
            "duplicate graph-node editor inventory name '{}'",
            registration.name()
        );
    }
}

#[cfg(test)]
mod editor_registration_tests {
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
        let first = GraphNodeEditorRegistration::definition::<FirstNode>("a");
        let second = GraphNodeEditorRegistration::definition::<SecondNode>("b");
        let mut registrations = vec![&second, &first];

        validate_graph_node_editor_registrations(&mut registrations);

        assert_eq!(registrations[0].stable_id(), "a");
        assert_eq!(registrations[1].stable_id(), "b");
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let registration = GraphNodeEditorRegistration::definition::<FirstNode>("duplicate");
        let mut registrations = vec![&registration, &registration];
        assert!(
            std::panic::catch_unwind(move || {
                validate_graph_node_editor_registrations(&mut registrations)
            })
            .is_err()
        );
    }
}
