//! Native capture-export service adapter.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, TryRecvError};

use platform_artifacts::ArtifactRepository;
use signal_capture_session::{
    CaptureSessionId, CaptureSessionRepository, CaptureSessionRepositoryConfig,
};

use crate::capture_export::{
    CaptureExportFormat, CaptureExportObserver, CaptureExportProgress, CaptureExportReport,
    export_finalized_capture,
};
use crate::service_contract::{
    CaptureExportCompletion, CaptureExportService, CaptureExportServiceError, CaptureExportStatus,
};

struct ExportObserver {
    cancellation: Arc<AtomicBool>,
    progress: Sender<CaptureExportProgress>,
}

impl CaptureExportObserver for ExportObserver {
    fn is_cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Relaxed)
    }

    fn on_progress(&mut self, progress: CaptureExportProgress) {
        let _ = self.progress.try_send(progress);
    }
}

struct ActiveExport {
    cancellation: Arc<AtomicBool>,
    progress: Receiver<CaptureExportProgress>,
    completion: Receiver<Result<CaptureExportReport, CaptureExportServiceError>>,
    worker: Option<JoinHandle<()>>,
}

struct NativeCaptureExportService {
    repository: CaptureSessionRepository,
    status: Option<CaptureExportStatus>,
    completion: Option<Result<CaptureExportCompletion, CaptureExportServiceError>>,
    active: Option<ActiveExport>,
}

impl NativeCaptureExportService {
    fn finish_active(&mut self) {
        if let Some(mut active) = self.active.take()
            && let Some(worker) = active.worker.take()
        {
            let _ = worker.join();
        }
    }
}

impl CaptureExportService for NativeCaptureExportService {
    fn start(
        &mut self,
        session_id: CaptureSessionId,
        format: CaptureExportFormat,
        destination: PathBuf,
    ) -> Result<(), CaptureExportServiceError> {
        if self.active.is_some() {
            return Err(CaptureExportServiceError::AlreadyActive);
        }
        let (capture, session_pin) = self
            .repository
            .open(session_id)
            .map_err(|error| CaptureExportServiceError::Capture(error.to_string()))?;
        let total_samples = capture.manifest().committed_samples;
        let cancellation = Arc::new(AtomicBool::new(false));
        let (progress_sender, progress) = crossbeam_channel::bounded(1);
        let (completion_sender, completion) = crossbeam_channel::bounded(1);
        let worker_cancellation = Arc::clone(&cancellation);
        let worker_destination = destination.clone();
        let worker = std::thread::Builder::new()
            .name("capture-export".into())
            .spawn(move || {
                let _session_pin = session_pin;
                let mut observer = ExportObserver {
                    cancellation: worker_cancellation,
                    progress: progress_sender,
                };
                let result =
                    export_finalized_capture(&capture, format, &worker_destination, &mut observer)
                        .map_err(CaptureExportServiceError::from);
                let _ = completion_sender.send(result);
            })
            .map_err(|error| CaptureExportServiceError::Executor(error.to_string()))?;
        self.completion = None;
        self.status = Some(CaptureExportStatus {
            format_label: format.descriptor().label.to_owned(),
            destination,
            samples_written: 0,
            total_samples,
            cancelling: false,
        });
        self.active = Some(ActiveExport {
            cancellation,
            progress,
            completion,
            worker: Some(worker),
        });
        Ok(())
    }

    fn status(&self) -> Option<&CaptureExportStatus> {
        self.status.as_ref()
    }

    fn take_completion(
        &mut self,
    ) -> Option<Result<CaptureExportCompletion, CaptureExportServiceError>> {
        self.completion.take()
    }

    fn request_cancel(&mut self) {
        let Some(active) = &self.active else {
            return;
        };
        active.cancellation.store(true, Ordering::Relaxed);
        if let Some(status) = &mut self.status {
            status.cancelling = true;
        }
    }

    fn poll(&mut self) {
        let mut latest_progress = None;
        if let Some(active) = &self.active {
            while let Ok(progress) = active.progress.try_recv() {
                latest_progress = Some(progress);
            }
        }
        if let Some(progress) = latest_progress
            && let Some(status) = &mut self.status
        {
            status.samples_written = progress.samples_written;
            status.total_samples = progress.total_samples;
        }
        let completion =
            self.active
                .as_ref()
                .and_then(|active| match active.completion.try_recv() {
                    Ok(completion) => Some(completion),
                    Err(TryRecvError::Empty) => None,
                    Err(TryRecvError::Disconnected) => {
                        Some(Err(CaptureExportServiceError::Executor(
                            "worker stopped without a result".into(),
                        )))
                    }
                });
        let Some(completion) = completion else {
            return;
        };
        self.finish_active();
        self.status = None;
        self.completion = Some(completion.map(|report| CaptureExportCompletion {
            destination: report.destination,
            warnings: report.warnings,
        }));
    }

    fn is_active(&self) -> bool {
        self.active.is_some()
    }

    fn reset(&mut self) {
        if self.active.is_none() {
            self.status = None;
            self.completion = None;
        }
    }
}

impl Drop for NativeCaptureExportService {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancellation.store(true, Ordering::Relaxed);
        }
        self.finish_active();
    }
}

/// Creates the native asynchronous capture-export service.
pub fn native_capture_export_service(
    artifact_repository: Arc<dyn ArtifactRepository>,
) -> Box<dyn CaptureExportService> {
    let repository =
        CaptureSessionRepository::new(CaptureSessionRepositoryConfig::new(artifact_repository))
            .expect("the live-capture artifact repository must be available for export");
    Box::new(NativeCaptureExportService {
        repository,
        status: None,
        completion: None,
        active: None,
    })
}
