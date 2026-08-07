//! Application-facing adapter for concrete raw-capture exporters.

use std::path::{Path, PathBuf};

use signal_capture_session::FinalizedCapture;

use super::errors::CaptureExportError;
use super::implementation::{
    CaptureExportObserver as RawCaptureExportObserver,
    CaptureExportProgress as RawCaptureExportProgress, CaptureExportRequest,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureExportFormat {
    Portable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureExportDescriptor {
    pub label: &'static str,
    pub extension: &'static str,
    pub dialog_title: &'static str,
    pub default_file_name: &'static str,
}

impl CaptureExportFormat {
    /// Returns user-facing dialog and filename metadata for this export format.
    pub const fn descriptor(self) -> CaptureExportDescriptor {
        match self {
            Self::Portable => CaptureExportDescriptor {
                label: "PulseView capture",
                extension: "sr",
                dialog_title: "Save Capture Data",
                default_file_name: "capture.sr",
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureExportProgress {
    pub samples_written: u64,
    pub total_samples: u64,
}

pub trait CaptureExportObserver {
    /// Returns whether the caller has requested cooperative cancellation.
    fn is_cancelled(&self) -> bool {
        false
    }

    /// Receives monotonic export progress updates.
    ///
    /// # Parameters
    /// - `_progress`: Number of samples exported relative to the capture total.
    fn on_progress(&mut self, _progress: CaptureExportProgress) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureExportReport {
    pub destination: PathBuf,
    pub samples_written: u64,
    pub encoded_bytes: u64,
    pub warnings: Vec<String>,
}

struct ObserverAdapter<'a>(&'a mut dyn CaptureExportObserver);

impl RawCaptureExportObserver for ObserverAdapter<'_> {
    fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }

    fn on_progress(&mut self, progress: RawCaptureExportProgress) {
        self.0.on_progress(CaptureExportProgress {
            samples_written: progress.samples_written,
            total_samples: progress.total_samples,
        });
    }
}

/// Exports a finalized capture using the requested concrete file format.
///
/// # Parameters
/// - `capture`: Finalized raw capture and metadata to encode.
/// - `format`: Output format selected by the UI.
/// - `destination`: Destination file path; the exporter overwrites it.
/// - `observer`: Cancellation and progress observer called during encoding.
pub fn export_finalized_capture(
    capture: &FinalizedCapture,
    format: CaptureExportFormat,
    destination: &Path,
    observer: &mut dyn CaptureExportObserver,
) -> Result<CaptureExportReport, CaptureExportError> {
    match format {
        CaptureExportFormat::Portable => {}
    }
    let request = CaptureExportRequest {
        destination: destination.to_owned(),
        overwrite: true,
    };
    let report = super::implementation::export_finalized_capture(
        capture,
        &request,
        &mut ObserverAdapter(observer),
    )?;
    Ok(CaptureExportReport {
        destination: report.destination,
        samples_written: report.samples_written,
        encoded_bytes: report.encoded_bytes,
        warnings: report
            .warnings
            .into_iter()
            .map(|warning| warning.message().to_owned())
            .collect(),
    })
}
