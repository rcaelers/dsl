//! Native streaming export of finalized logic-analyzer captures.
//!
//! The exporter writes finalized generic capture storage with explicit format,
//! progress, observer, and result contracts. Graph concerns, concrete processing
//! nodes, and UI workflow policy remain outside this crate. The native service
//! adapter implements the crate-owned application service contract at this
//! domain boundary.

mod capture_export;
mod service;
mod service_contract;

pub use capture_export::{
    CaptureExportDescriptor, CaptureExportError, CaptureExportFormat, CaptureExportObserver,
    CaptureExportProgress, CaptureExportReport, export_finalized_capture,
};
pub use service::native_capture_export_service;
pub use service_contract::{
    CaptureExportCompletion, CaptureExportService, CaptureExportServiceError, CaptureExportStatus,
};
