//! Native raw-capture export implementation and application-facing facade.

mod implementation;
mod presentation;

pub use presentation::{
    CaptureExportDescriptor, CaptureExportError, CaptureExportFormat, CaptureExportObserver,
    CaptureExportProgress, CaptureExportReport, export_finalized_capture,
};
