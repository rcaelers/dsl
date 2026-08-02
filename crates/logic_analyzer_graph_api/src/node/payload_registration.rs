use std::sync::Arc;

use signal_processing::{CollectedLaneRequest, PayloadAdapter};

use crate::node_support::{
    DefaultLanePresentationDescriptor, NodeBuildContext, PortKind, PortValue, ResolvedInput,
};

pub type PayloadRequestConfigurator =
    fn(CollectedLaneRequest, usize, &ResolvedInput, &dyn NodeBuildContext) -> CollectedLaneRequest;

pub struct PayloadRegistration {
    stable_id: &'static str,
    kind: fn() -> PortKind,
    adapter: fn() -> Arc<dyn PayloadAdapter>,
    presentation: fn() -> DefaultLanePresentationDescriptor,
    configure_request: PayloadRequestConfigurator,
    persistent_cache: bool,
}

impl PayloadRegistration {
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

    pub const fn stable_id(&self) -> &'static str {
        self.stable_id
    }

    pub fn kind(&self) -> PortKind {
        (self.kind)()
    }

    #[doc(hidden)]
    pub fn adapter(&self) -> Arc<dyn PayloadAdapter> {
        (self.adapter)()
    }

    #[doc(hidden)]
    pub fn presentation(&self) -> DefaultLanePresentationDescriptor {
        (self.presentation)()
    }

    #[doc(hidden)]
    pub const fn configure_request(&self) -> PayloadRequestConfigurator {
        self.configure_request
    }

    #[doc(hidden)]
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
