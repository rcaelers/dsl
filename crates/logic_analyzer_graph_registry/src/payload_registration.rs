use std::collections::HashSet;
use std::sync::Arc;

use logic_analyzer_graph_capabilities::node_support::{
    DefaultLanePresentationDescriptor, NodeBuildContext, PortKind, PortValue, ResolvedInput,
};
use signal_derived::{CollectedLaneRequest, PayloadAdapter};

/// Customizes one generated collected-lane request for a node output.
pub type PayloadRequestConfigurator =
    fn(CollectedLaneRequest, usize, &ResolvedInput, &dyn NodeBuildContext) -> CollectedLaneRequest;

/// Inventory submission associating a stable payload identity with collection behavior.
pub struct PayloadRegistration {
    stable_id: &'static str,
    kind: fn() -> PortKind,
    adapter: fn() -> Arc<dyn PayloadAdapter>,
    presentation: fn() -> DefaultLanePresentationDescriptor,
    configure_request: PayloadRequestConfigurator,
    persistent_cache: bool,
}

impl PayloadRegistration {
    /// Registers a collectable payload type without persistent indexed storage.
    pub const fn subscribable<T: PortValue>(
        stable_id: &'static str,
        adapter: fn() -> Arc<dyn PayloadAdapter>,
        presentation: fn() -> DefaultLanePresentationDescriptor,
    ) -> Self {
        Self::subscribable_with_request_configurator::<T>(
            stable_id,
            adapter,
            presentation,
            identity_request,
            false,
        )
    }

    /// Registers a collectable payload whose adapter supports persistent indexed storage.
    pub const fn subscribable_with_persistent_cache<T: PortValue>(
        stable_id: &'static str,
        adapter: fn() -> Arc<dyn PayloadAdapter>,
        presentation: fn() -> DefaultLanePresentationDescriptor,
    ) -> Self {
        Self::subscribable_with_request_configurator::<T>(
            stable_id,
            adapter,
            presentation,
            identity_request,
            true,
        )
    }

    /// Registers a collectable kind supplied through an explicit kind factory.
    pub const fn subscribable_kind(
        stable_id: &'static str,
        kind: fn() -> PortKind,
        adapter: fn() -> Arc<dyn PayloadAdapter>,
        presentation: fn() -> DefaultLanePresentationDescriptor,
    ) -> Self {
        Self {
            stable_id,
            kind,
            adapter,
            presentation,
            configure_request: identity_request,
            persistent_cache: false,
        }
    }

    /// Registers a collectable payload with explicit request customization.
    pub const fn subscribable_with_request_configurator<T: PortValue>(
        stable_id: &'static str,
        adapter: fn() -> Arc<dyn PayloadAdapter>,
        presentation: fn() -> DefaultLanePresentationDescriptor,
        configure_request: PayloadRequestConfigurator,
        persistent_cache: bool,
    ) -> Self {
        Self {
            stable_id,
            kind: PortKind::of::<T>,
            adapter,
            presentation,
            configure_request,
            persistent_cache,
        }
    }

    /// Returns the stable payload identity.
    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    /// Returns the registered runtime port kind.
    pub fn kind(&self) -> PortKind {
        (self.kind)()
    }

    /// Creates the registered payload adapter.
    pub fn adapter(&self) -> Arc<dyn PayloadAdapter> {
        (self.adapter)()
    }

    /// Creates the registered default presentation descriptor.
    pub fn presentation(&self) -> DefaultLanePresentationDescriptor {
        (self.presentation)()
    }

    /// Returns the registered request customizer.
    pub const fn configure_request(&self) -> PayloadRequestConfigurator {
        self.configure_request
    }

    /// Returns whether this payload supports persistent indexed caching.
    pub const fn persistent_cache(&self) -> bool {
        self.persistent_cache
    }
}

fn identity_request(
    request: CollectedLaneRequest,
    _member: usize,
    _input: &ResolvedInput,
    _ctx: &dyn NodeBuildContext,
) -> CollectedLaneRequest {
    request
}

inventory::collect!(PayloadRegistration);

/// Returns validated payload registrations in stable-ID order.
pub fn payload_registrations() -> Vec<&'static PayloadRegistration> {
    let mut registrations = inventory::iter::<PayloadRegistration>
        .into_iter()
        .collect::<Vec<_>>();
    validate_payload_registrations(&mut registrations);
    registrations
}

fn validate_payload_registrations(registrations: &mut Vec<&PayloadRegistration>) {
    registrations.sort_by_key(|registration| registration.stable_id());
    let mut stable_ids = HashSet::new();
    let mut type_ids = HashSet::new();
    for registration in registrations {
        assert!(
            !registration.stable_id().trim().is_empty(),
            "payload inventory contains an empty stable ID"
        );
        assert!(
            stable_ids.insert(registration.stable_id()),
            "duplicate payload inventory stable ID '{}'",
            registration.stable_id()
        );
        assert!(
            type_ids.insert(registration.kind().type_id()),
            "duplicate payload inventory type '{}'",
            registration.kind().name()
        );
    }
}

#[cfg(test)]
mod payload_registration_tests {
    use signal_capture::Sample;

    use super::*;

    fn test_presentation() -> DefaultLanePresentationDescriptor {
        DefaultLanePresentationDescriptor::new(
            logic_analyzer_graph_capabilities::node_support::LaneBadgeDescriptor::new(
                "T",
                [0, 0, 0],
            ),
            "org.logicconduit.registry-test.renderer/v1",
        )
    }

    #[test]
    fn registrations_are_stably_ordered_and_unique() {
        let registrations = payload_registrations();
        assert!(
            registrations
                .windows(2)
                .all(|pair| pair[0].stable_id() < pair[1].stable_id())
        );
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let registration = PayloadRegistration::subscribable::<Sample>(
            "org.logicconduit.registry-test.payload/v1",
            signal_derived::digital_payload_adapter,
            test_presentation,
        );
        let mut registrations = vec![&registration, &registration];
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                validate_payload_registrations(&mut registrations)
            }))
            .is_err()
        );
    }
}
