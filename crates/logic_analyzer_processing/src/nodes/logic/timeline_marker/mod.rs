//! Timeline-marker sources and conversions to event and level streams.

mod implementation;

pub use implementation::{
    MarkerRelation, TimelineMarkerRelation, TimelineMarkerSource, TimelineMarkerToTrigger,
    TimelineMarkerWindow,
};
