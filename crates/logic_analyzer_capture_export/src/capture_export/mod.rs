//! Native raw-capture export implementation and application-facing facade.

mod errors;
mod implementation;
mod presentation;

pub use errors::CaptureExportError;
pub use presentation::{
    CaptureExportDescriptor, CaptureExportFormat, CaptureExportObserver, CaptureExportProgress,
    CaptureExportReport, export_finalized_capture,
};
