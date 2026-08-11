//! UI-owned capture-provider port and application adapters.
//!
//! Prepared finite sources and active acquisition sources publish the same presentation and
//! artifact-readiness vocabulary through this module. Acquisition commands remain an optional
//! capability instead of becoming part of the generic data-provider surface.

#[cfg(test)]
mod architecture_tests;
mod contract;
mod live;
mod prepared;

pub(crate) use contract::{CaptureDataProvider, CapturePresentationUpdate, CaptureProviderPoll};
pub(crate) use live::LiveCaptureProvider;
pub(crate) use prepared::PreparedCaptureProvider;
