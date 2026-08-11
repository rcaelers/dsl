//! Execution-ready graph values without compiler or runtime lifecycle services.

mod payload_catalog_error;
mod types;

pub use payload_catalog_error::PayloadCatalogConfigurationError;
pub use types::{
    CapturePresentationDiscoveryError, CollectedOutputLane, CollectedOutputSubscription,
    CollectedTableSubscription, DiscoveredCapturePresentation, OutputSubscriptionPlan,
    ProcessingEdge, ProcessingGraph, ProcessingGraphError, ProcessingNode,
    ProcessingPayloadCatalog, ResolvedSamplingOverlay, SamplingOverlayCandidate,
};
