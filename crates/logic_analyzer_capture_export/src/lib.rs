//! Native streaming export of finalized logic-analyzer captures.
//!
//! The exporter writes finalized generic capture storage with explicit format,
//! progress, observer, and result contracts. Graph concerns, concrete processing
//! nodes, and UI policy remain outside this crate.

mod capture_export;

pub use capture_export::{
    CaptureExportDescriptor, CaptureExportFormat, CaptureExportObserver, CaptureExportProgress,
    CaptureExportReport, export_finalized_capture,
};
