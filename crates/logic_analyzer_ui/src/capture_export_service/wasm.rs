use std::path::PathBuf;

use signal_processing::CaptureSessionId;

use super::contract::{
    CaptureExportCompletion, CaptureExportFormat, CaptureExportService, CaptureExportStatus,
};

struct WebCaptureExportService;

impl CaptureExportService for WebCaptureExportService {
    fn start(
        &mut self,
        _session_id: CaptureSessionId,
        _format: CaptureExportFormat,
        _destination: PathBuf,
    ) -> Result<(), String> {
        Err("capture export is unavailable on this host".into())
    }

    fn status(&self) -> Option<&CaptureExportStatus> {
        None
    }

    fn take_completion(&mut self) -> Option<Result<CaptureExportCompletion, String>> {
        None
    }

    fn request_cancel(&mut self) {}

    fn poll(&mut self) {}

    fn is_active(&self) -> bool {
        false
    }

    fn reset(&mut self) {}
}

pub(crate) fn standard_capture_export_service() -> Box<dyn CaptureExportService> {
    Box::new(WebCaptureExportService)
}
