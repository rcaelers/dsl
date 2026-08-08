//! Portable live-acquisition lifecycle for concrete capture providers.

use std::error::Error as StdError;

use thiserror::Error;

use platform_runtime::{InlineWorkExecutor, WorkExecutor};

use super::implementation::{
    CaptureAcquisitionPhase, CaptureChunk, CaptureChunkWriter, CaptureCompletion,
    CaptureDataDelivery, CaptureEvent, CaptureEventPublishError, CaptureEventPublisher,
    CaptureFailure, CaptureFailureKind, CaptureHealth, CaptureProgress, CaptureSessionId,
    CaptureSessionState, CaptureStatus, CaptureWriteError,
};
use crate::capture_policy::{CaptureSessionPlan, CaptureStartMode};

/// Result alias for provider preparation and acquisition execution.
pub type AcquisitionResult<T> = Result<T, AcquisitionError>;
#[derive(Debug, Error)]
pub enum AcquisitionError {
    /// The request conflicts with provider capabilities or capture policy.
    #[error("invalid acquisition request: {0}")]
    InvalidRequest(#[source] Box<dyn StdError + Send + Sync>),
    /// An attempt was made to start an already-started acquisition.
    #[error("acquisition has already started")]
    AlreadyStarted,
    /// An operation requiring a started acquisition was requested too early.
    #[error("acquisition has not been started")]
    NotStarted,
    /// The provider does not implement the requested lifecycle operation.
    #[error("unsupported acquisition operation: {0}")]
    UnsupportedOperation(String),
    /// The authoritative capture writer rejected a chunk or finalization.
    #[error("capture writer failed: {0}")]
    Writer(#[from] CaptureWriteError),
    /// Publishing capture status to observers failed.
    #[error("capture status publication failed: {0}")]
    Event(#[from] CaptureEventPublishError),
    /// Transport communication with the acquisition device failed.
    #[error("acquisition transport failed: {0}")]
    Transport(#[source] Box<dyn StdError + Send + Sync>),
    /// The provider or device returned an invalid protocol response.
    #[error("acquisition protocol failed: {0}")]
    Protocol(String),
    /// Received data violated capture integrity guarantees.
    #[error("capture integrity was lost: {0}")]
    Integrity(String),
    /// The acquisition was cooperatively cancelled.
    #[error("acquisition was cancelled")]
    Cancelled,
    /// The host worker executing acquisition panicked.
    #[error("acquisition worker panicked")]
    WorkerPanicked,
    /// The host could not start the acquisition worker.
    #[error("acquisition worker could not be started: {0}")]
    WorkerStart(String),
    /// An uncategorized provider-internal failure occurred.
    #[error("acquisition failed: {0}")]
    Internal(String),
}

#[derive(Debug, Error)]
#[error("{0}")]
struct AcquisitionDiagnostic(String);

impl PartialEq for AcquisitionError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::UnsupportedOperation(left), Self::UnsupportedOperation(right))
            | (Self::Protocol(left), Self::Protocol(right))
            | (Self::Integrity(left), Self::Integrity(right))
            | (Self::WorkerStart(left), Self::WorkerStart(right))
            | (Self::Internal(left), Self::Internal(right)) => left == right,
            (Self::Writer(left), Self::Writer(right)) => left == right,
            (Self::Event(left), Self::Event(right)) => left == right,
            (Self::Transport(left), Self::Transport(right)) => {
                left.to_string() == right.to_string()
            }
            (Self::InvalidRequest(left), Self::InvalidRequest(right)) => {
                left.to_string() == right.to_string()
            }
            (Self::AlreadyStarted, Self::AlreadyStarted)
            | (Self::NotStarted, Self::NotStarted)
            | (Self::Cancelled, Self::Cancelled)
            | (Self::WorkerPanicked, Self::WorkerPanicked) => true,
            _ => false,
        }
    }
}

impl Eq for AcquisitionError {}

impl AcquisitionError {
    /// Retains a typed cause for an invalid provider request.
    pub fn invalid_request(error: impl StdError + Send + Sync + 'static) -> Self {
        Self::InvalidRequest(Box::new(error))
    }

    /// Adapts a provider that exposes only an invalid-request diagnostic.
    pub fn invalid_request_message(message: impl Into<String>) -> Self {
        Self::invalid_request(AcquisitionDiagnostic(message.into()))
    }

    /// Retains a typed transport failure raised by an acquisition provider.
    pub fn transport(error: impl StdError + Send + Sync + 'static) -> Self {
        Self::Transport(Box::new(error))
    }

    /// Adapts a provider that exposes only a transport diagnostic.
    pub fn transport_message(message: impl Into<String>) -> Self {
        Self::transport(AcquisitionDiagnostic(message.into()))
    }

    /// Converts this error into the persistent capture-failure category.
    pub fn failure_kind(&self) -> CaptureFailureKind {
        match self {
            Self::InvalidRequest(_) => CaptureFailureKind::InvalidRequest,
            Self::UnsupportedOperation(_) => CaptureFailureKind::InvalidRequest,
            Self::Writer(_) => CaptureFailureKind::Writer,
            Self::Transport(_) => CaptureFailureKind::Transport,
            Self::Protocol(_) => CaptureFailureKind::Protocol,
            Self::Integrity(_) => CaptureFailureKind::Integrity,
            Self::Cancelled => CaptureFailureKind::Cancelled,
            Self::AlreadyStarted
            | Self::NotStarted
            | Self::Event(_)
            | Self::WorkerPanicked
            | Self::WorkerStart(_)
            | Self::Internal(_) => CaptureFailureKind::Internal,
        }
    }
}

/// Dependencies supplied to a prepared acquisition without exposing store internals.
pub struct AcquisitionContext {
    session_id: CaptureSessionId,
    writer: Box<dyn CaptureChunkWriter>,
    events: Box<dyn CaptureEventPublisher>,
    work_executor: std::sync::Arc<dyn WorkExecutor>,
}

impl AcquisitionContext {
    /// Creates an acquisition context with the portable inline executor.
    ///
    /// # Parameters
    /// - `session_id`: Identity assigned to this capture session.
    /// - `writer`: Authority that commits ordered capture chunks.
    /// - `events`: Publisher used for lifecycle and progress notifications.
    pub fn new(
        session_id: CaptureSessionId,
        writer: Box<dyn CaptureChunkWriter>,
        events: Box<dyn CaptureEventPublisher>,
    ) -> Self {
        Self {
            session_id,
            writer,
            events,
            work_executor: std::sync::Arc::new(InlineWorkExecutor),
        }
    }

    /// Selects the host executor used by a prepared acquisition.
    ///
    /// # Parameters
    ///
    /// - `work_executor`: Host capability that runs long-lived acquisition work.
    pub fn with_work_executor(mut self, work_executor: std::sync::Arc<dyn WorkExecutor>) -> Self {
        self.work_executor = work_executor;
        self
    }

    /// Returns the host executor selected for this acquisition session.
    pub fn work_executor(&self) -> std::sync::Arc<dyn WorkExecutor> {
        std::sync::Arc::clone(&self.work_executor)
    }

    /// Returns the identity of the capture session being acquired.
    pub const fn session_id(&self) -> CaptureSessionId {
        self.session_id
    }

    /// Commits one session-owned chunk through the authoritative writer.
    ///
    /// # Parameters
    ///
    /// - `chunk`: Capture data belonging to this context's session.
    pub fn append(&mut self, chunk: CaptureChunk) -> AcquisitionResult<()> {
        if chunk.session_id() != self.session_id {
            return Err(AcquisitionError::invalid_request_message(format!(
                "chunk belongs to session {}, expected {}",
                chunk.session_id(),
                self.session_id
            )));
        }
        self.writer.append(chunk)?;
        Ok(())
    }

    /// Finalizes the authoritative writer after the provider stops producing chunks.
    pub fn finish_writer(&mut self) -> AcquisitionResult<()> {
        self.writer.finish()?;
        Ok(())
    }

    /// Publishes a lifecycle state and acquisition phase.
    ///
    /// # Parameters
    ///
    /// - `state`: New durable session state.
    /// - `phase`: Current acquisition phase within that state.
    pub fn publish_status(
        &mut self,
        state: CaptureSessionState,
        phase: CaptureAcquisitionPhase,
    ) -> AcquisitionResult<()> {
        self.events.publish(CaptureEvent::Status(CaptureStatus {
            session_id: self.session_id,
            state,
            phase,
        }))?;
        Ok(())
    }

    /// Publishes current acquisition progress.
    ///
    /// # Parameters
    ///
    /// - `progress`: Progress snapshot to expose to observers.
    pub fn publish_progress(&mut self, progress: CaptureProgress) -> AcquisitionResult<()> {
        self.events.publish(CaptureEvent::Progress {
            session_id: self.session_id,
            progress,
        })?;
        Ok(())
    }

    /// Publishes current acquisition health.
    ///
    /// # Parameters
    ///
    /// - `health`: Health snapshot to expose to observers.
    pub fn publish_health(&mut self, health: CaptureHealth) -> AcquisitionResult<()> {
        self.events.publish(CaptureEvent::Health {
            session_id: self.session_id,
            health,
        })?;
        Ok(())
    }

    /// Publishes the negotiated capture-session plan.
    ///
    /// # Parameters
    /// - `plan`: Effective policy and retention plan for this session.
    pub fn publish_plan(&mut self, plan: CaptureSessionPlan) -> AcquisitionResult<()> {
        self.events.publish(CaptureEvent::Plan {
            session_id: self.session_id,
            plan,
        })?;
        Ok(())
    }

    /// Publishes the sample position at which the trigger fired.
    ///
    /// # Parameters
    ///
    /// - `sample`: Absolute capture sample at the trigger boundary.
    pub fn publish_triggered(&mut self, sample: u64) -> AcquisitionResult<()> {
        self.events.publish(CaptureEvent::Triggered {
            session_id: self.session_id,
            sample,
        })?;
        Ok(())
    }

    /// Best-effort publishes a terminal failure and error status.
    ///
    /// # Parameters
    ///
    /// - `error`: Acquisition failure to classify and expose.
    pub fn publish_failure(&mut self, error: &AcquisitionError) {
        let _ = self
            .events
            .publish(CaptureEvent::Failed(CaptureFailure::new(
                self.session_id,
                error.failure_kind(),
                error.to_string(),
            )));
        let _ = self.events.publish(CaptureEvent::Status(CaptureStatus {
            session_id: self.session_id,
            state: CaptureSessionState::Error,
            phase: CaptureAcquisitionPhase::Finalizing,
        }));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AcquisitionOutcome {
    /// Identity of the finished session.
    pub session_id: CaptureSessionId,
    /// Number of samples accepted by the session.
    pub captured_samples: u64,
    /// Number of chunks committed by the provider.
    pub chunk_count: u64,
    /// Whether completion resulted from an explicit stop request.
    pub stopped: bool,
    /// Final completion reason.
    pub completion: CaptureCompletion,
}

/// Object-safe lifecycle boundary returned after a provider prepares a session.
pub trait PreparedAcquisition: Send {
    /// Returns the identity assigned during preparation.
    fn session_id(&self) -> CaptureSessionId;
    /// Starts acquisition and the provider-owned worker if needed.
    fn start(&mut self) -> AcquisitionResult<()>;
    /// Requests graceful stop while retaining captured data.
    fn request_stop(&self) -> AcquisitionResult<()>;
    /// Requests immediate abort when the provider supports it.
    fn request_abort(&self) -> AcquisitionResult<()> {
        Err(AcquisitionError::UnsupportedOperation("abort".into()))
    }
    /// Requests immediate trigger activation when the provider supports it.
    fn request_force_trigger(&self) -> AcquisitionResult<()> {
        Err(AcquisitionError::UnsupportedOperation(
            "force trigger".into(),
        ))
    }
    /// Non-blocking completion probe used by an acquisition supervisor so
    /// Stop remains available while Join runs off the UI thread.
    fn is_finished(&self) -> bool;
    /// Waits for provider work to finish and returns its terminal outcome.
    fn join(self: Box<Self>) -> AcquisitionResult<AcquisitionOutcome>;
}

/// A validated acquisition request that has not opened its transport yet.
///
/// Concrete capture devices implement this after validating their settings.
/// Hosts can inspect generic delivery facts and choose a start mode without
/// depending on the device implementation.
pub trait ConfiguredAcquisition: Send {
    /// Returns whether data is delivered by polling, callbacks, or a worker.
    fn data_delivery(&self) -> CaptureDataDelivery;
    /// Returns the physical capture window, in samples.
    fn capture_window_samples(&self) -> u64;
    /// Consumes validated settings and prepares a startable session.
    ///
    /// # Parameters
    /// - `context`: Session-owned writer, event publisher, and host executor.
    /// - `mode`: Start mode negotiated from policy and delivery capabilities.
    fn prepare(
        self: Box<Self>,
        context: AcquisitionContext,
        mode: CaptureStartMode,
    ) -> AcquisitionResult<Box<dyn PreparedAcquisition>>;
}

#[cfg(test)]
mod acquisition_error_tests {
    use std::error::Error as _;

    use super::super::validation::CaptureValidationError;
    use super::AcquisitionError;

    #[derive(Debug, thiserror::Error)]
    #[error("controlled acquisition transport failure")]
    struct ControlledTransportFailure;

    #[test]
    fn transport_error_retains_the_injected_provider_cause() {
        let error = AcquisitionError::transport(ControlledTransportFailure);

        assert!(error.source().unwrap().is::<ControlledTransportFailure>());
    }

    #[test]
    fn invalid_request_retains_the_capture_validation_cause() {
        let error =
            AcquisitionError::invalid_request(CaptureValidationError::CapabilitySettingMatrixEmpty);

        assert!(error.source().unwrap().is::<CaptureValidationError>());
    }
}
