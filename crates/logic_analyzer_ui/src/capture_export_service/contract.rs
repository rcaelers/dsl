use std::path::PathBuf;

use super::platform_contract::PlatformCaptureExportService;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureExportStatus {
    pub(crate) format_label: String,
    pub(crate) destination: PathBuf,
    pub(crate) samples_written: u64,
    pub(crate) total_samples: u64,
    pub(crate) cancelling: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureExportCompletion {
    pub(crate) destination: PathBuf,
    pub(crate) warnings: Vec<String>,
}

pub(crate) trait CaptureExportService: PlatformCaptureExportService {}
