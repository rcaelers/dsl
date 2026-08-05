//! Example compile-time plugin crate proving the graph, runtime, viewer, and
//! application-panel extension contracts with payloads owned outside the host crates.
//!
//! It is intentionally a reference implementation of the supported plugin surface,
//! not a dependency of the host's generic compiler, viewer, or UI infrastructure.

mod camera_frame;
mod pulse_measure;

pub use camera_frame::{CameraFrame, CameraFrameSocket, CameraFrameSource};
pub use pulse_measure::{PulseMeasure, PulseSocket, PulseWidth};

static LINK_ANCHOR: u8 = 0;

/// Linker anchor used by a host that enables this compile-time plugin.
#[inline(never)]
pub fn link() -> usize {
    std::ptr::addr_of!(LINK_ANCHOR) as usize
}

#[cfg(test)]
mod architecture_tests;
