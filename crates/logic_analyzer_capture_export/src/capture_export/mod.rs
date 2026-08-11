//! Native raw-capture export implementation and application-facing facade.

mod errors;
mod presentation;
mod streaming_export;

pub use errors::CaptureExportError;
pub use presentation::{
    CaptureExportDescriptor, CaptureExportFormat, CaptureExportObserver, CaptureExportProgress,
    CaptureExportReport, export_finalized_capture,
};
