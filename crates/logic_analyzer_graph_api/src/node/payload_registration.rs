use std::sync::Arc;

use signal_processing::{CollectedLaneRequest, PayloadAdapter};

use crate::node_support::{
    DefaultLanePresentationDescriptor, NodeBuildContext, PortKind, PortValue, ResolvedInput,
};

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
    ///
    /// # Parameters
    /// - `stable_id`: Stable payload identity used in persisted and plugin contracts.
    /// - `adapter`: Factory for the payload's collection adapter.
    /// - `presentation`: Factory for the payload's default lane presentation.
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

    /// Registers a collectable payload whose adapter supports the generic
    /// repository-backed indexed-store contract.
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

    /// Registers a collectable kind whose payload type is owned below the
    /// graph-plugin layer and is therefore supplied as an explicit kind factory.
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

    /// Registers a collectable payload type with explicit request customization.
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

    /// Returns the stable payload identifier used across persisted and plugin contracts.
    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    /// Returns the runtime port kind produced by this payload registration.
    pub fn kind(&self) -> PortKind {
        (self.kind)()
    }

    #[doc(hidden)]
    /// Creates the registered payload adapter.
    pub fn adapter(&self) -> Arc<dyn PayloadAdapter> {
        (self.adapter)()
    }

    #[doc(hidden)]
    /// Creates the registered default lane presentation.
    pub fn presentation(&self) -> DefaultLanePresentationDescriptor {
        (self.presentation)()
    }

    #[doc(hidden)]
    /// Returns the registered collected-lane request customizer.
    pub const fn configure_request(&self) -> PayloadRequestConfigurator {
        self.configure_request
    }

    #[doc(hidden)]
    /// Returns whether the payload supports persistent indexed caching.
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
