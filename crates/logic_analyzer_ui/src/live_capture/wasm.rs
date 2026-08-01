use logic_analyzer_graph_compiler::DiscoveredLiveCaptureFeature;
use signal_processing::CaptureStartMode;

use super::implementation::{
    CaptureAnalysisAttachment, CaptureCoordinatorContract, CaptureReplayAttachment,
    CaptureSessionStatus, CaptureWaveformUpdate,
};
use crate::capture_export_service::{
    CaptureExportCompletion, CaptureExportService, CaptureExportStatus,
};

pub(crate) struct CaptureCoordinator {
    export_service: Box<dyn CaptureExportService>,
}

impl CaptureCoordinator {
    pub(crate) fn configured(
        _max_recent_sessions: usize,
        _max_total_bytes: u64,
        _capture_session_directory: Option<std::path::PathBuf>,
        _work_executor: std::sync::Arc<dyn signal_processing::WorkExecutor>,
        export_service: Box<dyn CaptureExportService>,
    ) -> Self {
        Self { export_service }
    }

    pub(crate) fn start_with_graph(
        &mut self,
        _feature: DiscoveredLiveCaptureFeature,
        _graph: &node_graph::GraphState,
        _mode: CaptureStartMode,
    ) -> Result<(), String> {
        Err(Self::backend_unavailable_reason().into())
    }

    pub(crate) fn export_status(&self) -> Option<&CaptureExportStatus> {
        self.export_service.status()
    }

    pub(crate) fn take_export_notice(&mut self) -> Option<Result<CaptureExportCompletion, String>> {
        self.export_service.take_completion()
    }

    pub(crate) fn request_cancel_export(&mut self) {
        self.export_service.request_cancel();
    }

    pub(crate) fn clear_completed(&mut self) {
        self.export_service.reset();
    }
}

impl CaptureCoordinatorContract for CaptureCoordinator {
    fn backend_available() -> bool {
        false
    }

    fn backend_unavailable_reason() -> &'static str {
        "Live capture is not available in this web build"
    }

    fn request_stop(&mut self) {}

    fn request_abort(&mut self) -> Result<(), String> {
        Err(Self::backend_unavailable_reason().into())
    }

    fn request_force_trigger(&mut self) -> Result<(), String> {
        Err(Self::backend_unavailable_reason().into())
    }

    fn set_graph_processed_samples(&mut self, _processed_samples: Option<u64>) {}

    fn poll(&mut self) {
        self.export_service.poll();
    }

    fn status(&self) -> Option<&CaptureSessionStatus> {
        None
    }

    fn take_waveform_update(&mut self) -> Option<CaptureWaveformUpdate> {
        None
    }

    fn take_analysis_attachment(&mut self) -> Option<CaptureAnalysisAttachment> {
        None
    }

    fn replay_source_node(&self) -> Option<node_graph::NodeId> {
        None
    }

    fn create_replay_attachment(&self) -> Result<Option<CaptureReplayAttachment>, String> {
        Ok(None)
    }

    fn is_active(&self) -> bool {
        false
    }
}
