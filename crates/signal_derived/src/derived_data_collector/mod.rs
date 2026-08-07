//! Presentation-neutral collection of typed derived streams into retained storage.

mod catalog;
mod collector;
mod digital;
mod indexed;
mod number;
mod storage;
#[cfg(test)]
mod tests;
mod text;
mod timestamp_event;
mod word;

pub use catalog::{DerivedLanes, OpaqueCollectedLane};
pub use collector::{
    DEFAULT_DERIVED_DATA_MAX_ENTRIES, DerivedDataCollector, DerivedDataCollectorMetrics,
    DerivedDataCollectorMetricsSnapshot, DerivedDataRetention,
};
pub use digital::{DigitalLaneSnapshot, digital_payload_adapter};
pub use number::{NumberLaneSnapshot, number_payload_adapter};
pub use text::{TextLaneSnapshot, text_payload_adapter};
pub use timestamp_event::{TimestampEventLaneSnapshot, timestamp_event_payload_adapter};
pub use word::{
    CollectedWordLaneOptions, CollectedWordLaneQuery, IndexedAnnotationLane, WordLaneSnapshot,
    built_in_word_lane_ingestor, word_payload_adapter,
};
