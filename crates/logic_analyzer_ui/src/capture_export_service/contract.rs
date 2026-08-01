use std::path::PathBuf;

use signal_processing::CaptureSessionId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureExportStatus {
    pub format_label: String,
    pub destination: PathBuf,
    pub samples_written: u64,
    pub total_samples: u64,
    pub cancelling: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureExportCompletion {
    pub destination: PathBuf,
    pub warnings: Vec<String>,
}

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

pub trait CaptureExportService {
    fn start(
        &mut self,
        session_id: CaptureSessionId,
        format: CaptureExportFormat,
        destination: PathBuf,
    ) -> Result<(), String>;

    fn status(&self) -> Option<&CaptureExportStatus>;

    fn take_completion(&mut self) -> Option<Result<CaptureExportCompletion, String>>;

    fn request_cancel(&mut self);

    fn poll(&mut self);

    fn is_active(&self) -> bool;

    fn reset(&mut self);
}
