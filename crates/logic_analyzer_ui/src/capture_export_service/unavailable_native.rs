use std::path::PathBuf;

use signal_processing::{CaptureSessionId, NativeCaptureSessionRepository};

use super::contract::{CaptureExportCompletion, CaptureExportService, CaptureExportStatus};
use super::platform_contract::{CaptureExportFormat, PlatformCaptureExportService};

struct UnavailableCaptureExportService;

impl PlatformCaptureExportService for UnavailableCaptureExportService {
    fn start(
        &mut self,
        _session_id: CaptureSessionId,
        _format: CaptureExportFormat,
        _destination: PathBuf,
    ) -> Result<(), String> {
        Err("native capture export is not enabled".into())
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

impl CaptureExportService for UnavailableCaptureExportService {}

pub(crate) fn standard_capture_export_service(
    _repository: NativeCaptureSessionRepository,
) -> Box<dyn CaptureExportService> {
    Box::new(UnavailableCaptureExportService)
}
