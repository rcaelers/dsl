use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use signal_processing::CaptureSessionId;

use super::contract::{
    CaptureExportCompletion, CaptureExportFormat, CaptureExportService, CaptureExportStatus,
};

enum ScriptedEvent {
    Progress {
        samples_written: u64,
        total_samples: u64,
    },
    Complete(Result<Vec<String>, String>),
}

#[derive(Default)]
struct SharedState {
    starts: Vec<(CaptureSessionId, CaptureExportFormat, PathBuf)>,
    cancel_requests: usize,
    start_error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ScriptedCaptureExportControl {
    shared: Arc<Mutex<SharedState>>,
}

impl ScriptedCaptureExportControl {
    pub(crate) fn starts(&self) -> Vec<(CaptureSessionId, CaptureExportFormat, PathBuf)> {
        self.shared.lock().unwrap().starts.clone()
    }

    pub(crate) fn cancel_requests(&self) -> usize {
        self.shared.lock().unwrap().cancel_requests
    }

    fn fail_start(&self, message: impl Into<String>) {
        self.shared.lock().unwrap().start_error = Some(message.into());
    }
}

struct ScriptedCaptureExportService {
    shared: Arc<Mutex<SharedState>>,
    events: VecDeque<ScriptedEvent>,
    status: Option<CaptureExportStatus>,
    completion: Option<Result<CaptureExportCompletion, String>>,
}

impl CaptureExportService for ScriptedCaptureExportService {
    fn start(
        &mut self,
        session_id: CaptureSessionId,
        format: CaptureExportFormat,
        destination: PathBuf,
    ) -> Result<(), String> {
        let mut shared = self.shared.lock().unwrap();
        if let Some(error) = shared.start_error.take() {
            return Err(error);
        }
        shared
            .starts
            .push((session_id, format, destination.clone()));
        drop(shared);
        self.completion = None;
        self.status = Some(CaptureExportStatus {
            format_label: format.descriptor().label.to_owned(),
            destination,
            samples_written: 0,
            total_samples: 128,
            cancelling: false,
        });
        Ok(())
    }

    fn status(&self) -> Option<&CaptureExportStatus> {
        self.status.as_ref()
    }

    fn take_completion(&mut self) -> Option<Result<CaptureExportCompletion, String>> {
        self.completion.take()
    }

    fn request_cancel(&mut self) {
        if let Some(status) = &mut self.status {
            status.cancelling = true;
            self.shared.lock().unwrap().cancel_requests += 1;
        }
    }

    fn poll(&mut self) {
        if self.status.is_none() {
            return;
        }
        let Some(event) = self.events.pop_front() else {
            return;
        };
        match event {
            ScriptedEvent::Progress {
                samples_written,
                total_samples,
            } => {
                if let Some(status) = &mut self.status {
                    status.samples_written = samples_written;
                    status.total_samples = total_samples;
                }
            }
            ScriptedEvent::Complete(result) => {
                let destination = self
                    .status
                    .take()
                    .expect("scripted completion requires an active export")
                    .destination;
                self.completion = Some(result.map(|warnings| CaptureExportCompletion {
                    destination,
                    warnings,
                }));
            }
        }
    }

    fn is_active(&self) -> bool {
        self.status.is_some()
    }

    fn reset(&mut self) {
        if self.status.is_none() {
            self.completion = None;
        }
    }
}

pub(crate) fn scripted_capture_export_service()
-> (Box<dyn CaptureExportService>, ScriptedCaptureExportControl) {
    scripted_service_with_events([
        ScriptedEvent::Progress {
            samples_written: 64,
            total_samples: 128,
        },
        ScriptedEvent::Complete(Ok(Vec::new())),
    ])
}

fn scripted_service_with_events(
    events: impl IntoIterator<Item = ScriptedEvent>,
) -> (Box<dyn CaptureExportService>, ScriptedCaptureExportControl) {
    let shared = Arc::new(Mutex::new(SharedState::default()));
    (
        Box::new(ScriptedCaptureExportService {
            shared: Arc::clone(&shared),
            events: events.into_iter().collect(),
            status: None,
            completion: None,
        }),
        ScriptedCaptureExportControl { shared },
    )
}

#[test]
fn scripted_service_controls_progress_cancellation_and_completion_without_host_io() {
    let (mut service, control) = scripted_capture_export_service();
    let destination = PathBuf::from("capture.sr");
    service
        .start(
            CaptureSessionId::new(7),
            CaptureExportFormat::Portable,
            destination.clone(),
        )
        .unwrap();
    service.poll();
    assert_eq!(service.status().unwrap().samples_written, 64);
    service.request_cancel();
    assert!(service.status().unwrap().cancelling);
    assert_eq!(control.cancel_requests(), 1);
    service.poll();
    assert_eq!(
        service.take_completion(),
        Some(Ok(CaptureExportCompletion {
            destination,
            warnings: Vec::new(),
        }))
    );
}

#[test]
fn scripted_service_controls_start_and_worker_failures() {
    let (mut service, control) = scripted_capture_export_service();
    control.fail_start("executor unavailable");
    assert_eq!(
        service.start(
            CaptureSessionId::new(8),
            CaptureExportFormat::Portable,
            PathBuf::from("capture.sr"),
        ),
        Err("executor unavailable".into())
    );

    let (mut service, _) =
        scripted_service_with_events([ScriptedEvent::Complete(Err("encoding failed".into()))]);
    service
        .start(
            CaptureSessionId::new(9),
            CaptureExportFormat::Portable,
            PathBuf::from("capture.sr"),
        )
        .unwrap();
    service.poll();
    assert_eq!(
        service.take_completion(),
        Some(Err("encoding failed".into()))
    );
}
