use super::contract::CaptureExportService;
use super::platform_contract::PlatformCaptureExportService;

struct WebCaptureExportService;

impl PlatformCaptureExportService for WebCaptureExportService {}

impl CaptureExportService for WebCaptureExportService {}

pub(crate) fn standard_capture_export_service() -> Box<dyn CaptureExportService> {
    Box::new(WebCaptureExportService)
}
