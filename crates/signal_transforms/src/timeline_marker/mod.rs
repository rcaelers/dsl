//! # `timeline_marker`
//!
//! ## Responsibility
//!
//! This module owns processing nodes that introduce, convert, or relate timeline-marker runtime values.
//!
//! ## Boundaries
//!
//! It does not own host cursor state, graph persistence, or marker rendering. The graph capabilities carries
//! the neutral marker references and the UI supplies host positions.

//! Timeline-marker sources and conversions to event and level streams.
//!
//! This module owns the runtime representation and stream conversions, while
//! editor actions and viewer presentation use explicit graph/UI contracts.

mod transforms;

pub use transforms::{
    MarkerRelation, TimelineMarkerRelation, TimelineMarkerSource, TimelineMarkerToEvent,
    TimelineMarkerWindow,
};
