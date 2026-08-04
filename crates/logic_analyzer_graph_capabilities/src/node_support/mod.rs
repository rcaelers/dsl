//! # `logic_analyzer_graph_capabilities::node_support`
//!
//! ## Responsibility
//!
//! This namespace owns value contracts supplied to graph-node implementations: port identities,
//! resolved inputs, restricted build context, state decoding, and presentation/capture descriptors.
//!
//! ## Boundaries
//!
//! It contains no editor widget, compiler lifecycle, concrete node behavior, target selection, or host
//! path handling. Descriptors carry stable metadata rather than requiring generic consumers to infer
//! behavior from display names.

//! Values and restricted build services supplied to graph-node implementations.
//!
//! The values describe ports, resolved inputs, capture identity, and neutral
//! presentation contracts. [`NodeBuildContext`] is the only plugin-visible
//! materialization service; it does not expose compiler implementation state.

mod contracts;
mod port;

pub use contracts::{
    CaptureCacheIdentity, CapturePresentation, CapturePresentationSignal, DecoderTableCellMode,
    DecoderTableColumnDescriptor, DefaultLanePresentationDescriptor, LaneBadgeDescriptor,
    LanePresentationDescriptor, LiveCaptureEdit, NodeBuildContext, ResolvedInput, ResolvedInputs,
    RetainedWordSamplingSource, SamplingOverlayDescriptor, SimpleTriggerChannel,
    SourceDataLifecycle, SourceDataLifecycleKind, TimelineMarkerDescriptor, TimelineMarkerEdit,
    TimelineMarkerReference, TimelineMarkerReferenceBindingDescriptor,
    TimelineMarkerReferenceBindingEdit, TimelineMarkerReferenceChoice, TriggerConfigurationFeature,
    ViewerOutputControl, ViewerOutputPanelAction, ViewerOutputPanelEntry, ViewerOutputPanelModel,
    parse_state,
};
pub use port::{PortKind, PortValue};
