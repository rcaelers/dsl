//! Application-facing projection of capture acquisition state.
//!
//! This module owns the current status and ordered state history behind
//! `CaptureStatusProjection`. It consumes provider-neutral events and terminal capture outcomes.
//! It does not schedule acquisition work, send commands, retain artifacts, export sessions, or
//! format widgets.

use signal_capture_session::{
    CaptureAcquisitionPhase, CaptureCompletion, CaptureEvent, CaptureHealth, CaptureProgress,
    CaptureSessionId, CaptureSessionOutcome, CaptureSessionPlan, CaptureSessionState,
};

use super::contract::CaptureSessionStatus;

/// Input needed to establish the UI projection for one newly started capture.
pub(crate) struct CaptureStartProjection {
    pub(crate) session_id: CaptureSessionId,
    pub(crate) source_node: node_graph::api::NodeId,
    pub(crate) source_title: String,
    pub(crate) commands: signal_capture_session::CaptureCommandCapabilities,
    pub(crate) session_plan: Option<CaptureSessionPlan>,
    pub(crate) recording_origin: Option<u64>,
}

/// Owns the capture-session state exposed through `CaptureCoordinatorContract`.
///
/// The projection accepts acquisition events and terminal outcomes in order and records each
/// distinct state once. It does not schedule acquisition work, persist capture artifacts, or own
/// command channels.
pub(crate) struct CaptureStatusProjection {
    current: Option<CaptureSessionStatus>,
    state_history: Vec<CaptureSessionState>,
}

impl CaptureStatusProjection {
    pub(crate) fn new() -> Self {
        Self {
            current: None,
            state_history: Vec::new(),
        }
    }

    pub(crate) fn start(&mut self, start: CaptureStartProjection) {
        self.current = Some(CaptureSessionStatus {
            session_id: start.session_id,
            source_node: start.source_node,
            source_title: start.source_title,
            state: CaptureSessionState::Preparing,
            phase: CaptureAcquisitionPhase::Preparing,
            progress: CaptureProgress::default(),
            health: CaptureHealth::default(),
            commands: start.commands,
            session_plan: start.session_plan,
            trigger_sample: None,
            recording_origin: start.recording_origin,
            outcome: CaptureSessionOutcome::InProgress,
            completion: None,
            error: None,
        });
        self.state_history.clear();
        self.record_state(CaptureSessionState::Preparing);
    }

    pub(crate) fn clear(&mut self) {
        self.current = None;
    }

    pub(crate) fn status(&self) -> Option<&CaptureSessionStatus> {
        self.current.as_ref()
    }

    pub(crate) fn session_id(&self) -> Option<CaptureSessionId> {
        self.current.as_ref().map(|status| status.session_id)
    }

    pub(crate) fn supports_orderly_stop(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|status| status.commands.orderly_stop)
    }

    pub(crate) fn ensure_abort_supported(&self) -> Result<(), String> {
        let status = self
            .current
            .as_ref()
            .ok_or_else(|| "capture status is unavailable".to_owned())?;
        if status.commands.abort {
            Ok(())
        } else {
            Err("this capture source does not support Abort".into())
        }
    }

    pub(crate) fn ensure_force_trigger_supported(&self) -> Result<(), String> {
        let status = self
            .current
            .as_ref()
            .ok_or_else(|| "capture status is unavailable".to_owned())?;
        if status.state != CaptureSessionState::Armed {
            return Err("Force Trigger is available only while capture is armed".into());
        }
        if status.commands.force_trigger {
            Ok(())
        } else {
            Err("this capture source does not support Force Trigger".into())
        }
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|status| status.state == CaptureSessionState::Recording)
    }

    pub(crate) fn mark_stopping(&mut self) {
        let Some(status) = &mut self.current else {
            return;
        };
        status.state = CaptureSessionState::Stopping;
        status.phase = CaptureAcquisitionPhase::Finalizing;
        self.record_state(CaptureSessionState::Stopping);
    }

    /// Applies one acquisition event to the application-facing projection.
    pub(crate) fn apply_event(&mut self, event: CaptureEvent, stop_requested: bool) {
        let Some(status) = &mut self.current else {
            return;
        };
        let mut next_state = None;
        match event {
            CaptureEvent::Status(event) if event.session_id == status.session_id => {
                if stop_requested
                    && !matches!(
                        event.state,
                        CaptureSessionState::Stopping
                            | CaptureSessionState::Complete
                            | CaptureSessionState::Error
                    )
                {
                    return;
                }
                status.state = event.state;
                status.phase = event.phase;
                next_state = Some(event.state);
            }
            CaptureEvent::Progress {
                session_id,
                progress,
            } if session_id == status.session_id => status.progress = progress,
            CaptureEvent::Health { session_id, health } if session_id == status.session_id => {
                status.health = health;
            }
            CaptureEvent::Plan { session_id, plan } if session_id == status.session_id => {
                status.session_plan = Some(plan);
            }
            CaptureEvent::Triggered { session_id, sample } if session_id == status.session_id => {
                status.state = CaptureSessionState::Triggered;
                status.trigger_sample = Some(sample);
                status.recording_origin = Some(sample);
                status.session_plan = status
                    .session_plan
                    .take()
                    .map(|plan| plan.with_actual_trigger_sample(sample));
                next_state = Some(CaptureSessionState::Triggered);
            }
            CaptureEvent::Failed(failure) if failure.session_id == status.session_id => {
                status.state = CaptureSessionState::Error;
                status.phase = CaptureAcquisitionPhase::Finalizing;
                status.outcome = CaptureSessionOutcome::Incomplete;
                status.error = Some(failure.message);
                next_state = Some(CaptureSessionState::Error);
            }
            _ => {}
        }
        if let Some(state) = next_state {
            self.record_state(state);
        }
    }

    pub(crate) fn complete(
        &mut self,
        session_plan: Option<CaptureSessionPlan>,
        outcome: CaptureSessionOutcome,
        completion: Option<CaptureCompletion>,
    ) {
        if let Some(status) = &mut self.current {
            status.state = CaptureSessionState::Complete;
            status.phase = CaptureAcquisitionPhase::Finalizing;
            status.session_plan = session_plan;
            status.outcome = outcome;
            status.completion = completion;
        }
        self.record_state(CaptureSessionState::Complete);
    }

    pub(crate) fn fail(&mut self, error: String) {
        if let Some(status) = &mut self.current {
            status.state = CaptureSessionState::Error;
            status.phase = CaptureAcquisitionPhase::Finalizing;
            status.outcome = CaptureSessionOutcome::Incomplete;
            status.error = Some(error);
        }
        self.record_state(CaptureSessionState::Error);
    }

    pub(crate) fn report_error(&mut self, error: String) {
        if let Some(status) = &mut self.current {
            status.error = Some(error);
        }
    }

    pub(crate) fn set_graph_processed_samples(&mut self, processed_samples: Option<u64>) {
        let Some(status) = &mut self.current else {
            return;
        };
        status.health.graph_lag_samples = processed_samples.and_then(|processed| {
            status
                .recording_origin
                .zip(status.progress.captured_samples)
                .map(|(origin, captured)| captured.saturating_sub(origin).saturating_sub(processed))
        });
    }

    pub(crate) fn graph_editing_enabled(&self, acquisition_active: bool) -> bool {
        !acquisition_active || self.is_recording()
    }

    #[cfg(test)]
    pub(crate) fn state_history(&self) -> &[CaptureSessionState] {
        &self.state_history
    }

    fn record_state(&mut self, state: CaptureSessionState) {
        if self.state_history.last().copied() != Some(state) {
            self.state_history.push(state);
        }
    }
}
