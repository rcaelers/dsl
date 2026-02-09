//! Core data types for signal processing

use std::fmt;

/// Sample representing a signal value at a specific time
/// 
/// This is a run-length encoded representation that sends only when a signal changes,
/// dramatically reducing bandwidth for signals that don't toggle frequently.
/// 
/// The value remains constant until the next Sample arrives. Duration is determined
/// by the timestamp of the next sample (next.start_time - current.start_time).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    /// Channel value at this timestamp
    pub value: bool,
    /// Timestamp in nanoseconds when this value started
    pub start_time: u64,
}

impl Sample {
    /// Create a new sample
    pub fn new(value: bool, start_time: u64) -> Self {
        Self {
            value,
            start_time,
        }
    }
}

impl fmt::Display for Sample {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Sample[v={}, t={}]",
            self.value,
            self.start_time
        )
    }
}
