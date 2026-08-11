//! Neutral immutable processing-graph contract shared by graph producers and consumers.

mod plan;

pub use plan::{
    CapturePresentationDiscoveryError, CollectedOutputLane, CollectedOutputSubscription,
    CollectedTableSubscription, DiscoveredCapturePresentation, OutputSubscriptionPlan,
    PayloadCatalogConfigurationError, ProcessingEdge, ProcessingGraph, ProcessingGraphError,
    ProcessingNode, ProcessingPayloadCatalog, ResolvedSamplingOverlay, SamplingOverlayCandidate,
};
