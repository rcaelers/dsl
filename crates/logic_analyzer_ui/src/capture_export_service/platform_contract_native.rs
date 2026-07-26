use std::path::PathBuf;

use signal_processing::CaptureSessionId;

use super::contract::{CaptureExportCompletion, CaptureExportStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaptureExportFormat {
    Portable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureExportDescriptor {
    pub(crate) label: &'static str,
    pub(crate) extension: &'static str,
    pub(crate) dialog_title: &'static str,
    pub(crate) default_file_name: &'static str,
}

impl CaptureExportFormat {
    pub(crate) const fn descriptor(self) -> CaptureExportDescriptor {
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

pub(crate) trait PlatformCaptureExportService {
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
