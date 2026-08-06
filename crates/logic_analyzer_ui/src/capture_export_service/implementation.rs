use std::path::PathBuf;

use logic_analyzer_capture_export::{
    CaptureExportCompletion, CaptureExportFormat, CaptureExportService, CaptureExportStatus,
};
use signal_capture_session::CaptureSessionId;

struct UnavailableCaptureExportService;

impl CaptureExportService for UnavailableCaptureExportService {
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

/// Returns an explicit unavailable implementation for hosts without export support.
pub fn unavailable_capture_export_service() -> Box<dyn CaptureExportService> {
    Box::new(UnavailableCaptureExportService)
}
