use node_graph::api::{GraphState, NodeId};
use signal_capture::CaptureIndex;
use signal_capture_session::{
    CaptureAcquisitionPhase, CaptureCommandCapabilities, CaptureCompletion, CaptureHealth,
    CaptureProgress, CaptureProviderCapabilities, CaptureSessionId, CaptureSessionOutcome,
    CaptureSessionPlan, CaptureSessionState,
};
use signal_runtime::ProcessNode;

use super::error::CaptureCoordinatorError;

/// Outer `Option` on the coordinator method means "no update"; this inner
/// option carries either a new growing index or an explicit detach.
pub(crate) type CaptureWaveformUpdate = Option<Box<dyn CaptureIndex>>;

pub(crate) struct CaptureAnalysisAttachment {
    pub(crate) source_node: NodeId,
    pub(crate) process: Box<dyn ProcessNode>,
}

/// Fresh source process for re-analyzing one immutable finalized session.
pub(crate) struct CaptureReplayAttachment {
    pub(crate) source_node: NodeId,
    pub(crate) process: Box<dyn ProcessNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CaptureAvailability {
    Available {
        source_node: NodeId,
        source_title: String,
        has_trigger_program: bool,
        capabilities: CaptureProviderCapabilities,
        session_plan: Option<Box<CaptureSessionPlan>>,
    },
    Unavailable {
        reason: String,
    },
}

impl CaptureAvailability {
    pub(crate) fn reason(&self) -> Option<&str> {
        match self {
            Self::Available { .. } => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

pub(crate) trait CaptureFeatureDiscovery {
    fn discover_capture_availability(&self, graph: &GraphState) -> CaptureAvailability;
}

pub(crate) fn capture_availability<Discovery>(
    graph: &GraphState,
    discovery: &Discovery,
    backend_unavailable_reason: Option<&str>,
) -> CaptureAvailability
where
    Discovery: CaptureFeatureDiscovery + ?Sized,
{
    if let Some(reason) = backend_unavailable_reason {
        return CaptureAvailability::Unavailable {
            reason: reason.into(),
        };
    }
    discovery.discover_capture_availability(graph)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CaptureSessionStatus {
    pub(crate) session_id: CaptureSessionId,
    pub(crate) source_node: NodeId,
    pub(crate) source_title: String,
    pub(crate) state: CaptureSessionState,
    pub(crate) phase: CaptureAcquisitionPhase,
    pub(crate) progress: CaptureProgress,
    pub(crate) health: CaptureHealth,
    pub(crate) commands: CaptureCommandCapabilities,
    pub(crate) session_plan: Option<CaptureSessionPlan>,
    pub(crate) trigger_sample: Option<u64>,
    pub(crate) recording_origin: Option<u64>,
    pub(crate) outcome: CaptureSessionOutcome,
    pub(crate) completion: Option<CaptureCompletion>,
    pub(crate) error: Option<String>,
}

#[derive(Clone)]
pub(crate) struct PreparedConfigurationEpoch {
    pub(crate) epoch_id: u64,
    pub(crate) source_sample: u64,
    pub(crate) boundary: signal_runtime::ConfigurationBoundary,
    pub(crate) graph: node_graph::api::GraphState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfigurationEpochResolution {
    Applied,
    Deferred(String),
    Failed(String),
}

pub(crate) trait CaptureCoordinatorContract {
    fn backend_unavailable_reason(&self) -> Option<&'static str>;
    fn request_stop(&mut self);
    fn request_abort(&mut self) -> Result<(), CaptureCoordinatorError>;
    fn request_force_trigger(&mut self) -> Result<(), CaptureCoordinatorError>;
    fn set_graph_processed_samples(&mut self, processed_samples: Option<u64>);
    fn poll(&mut self);
    fn status(&self) -> Option<&CaptureSessionStatus>;
    fn take_waveform_update(&mut self) -> Option<CaptureWaveformUpdate>;
    fn take_analysis_attachment(&mut self) -> Option<CaptureAnalysisAttachment>;
    fn request_configuration_epoch(
        &mut self,
        _graph: node_graph::api::GraphState,
    ) -> Result<(), CaptureCoordinatorError> {
        Err(CaptureCoordinatorError::policy(
            "live configuration epochs are unavailable on this platform",
        ))
    }
    fn take_configuration_epoch_preparation(
        &mut self,
    ) -> Option<Result<PreparedConfigurationEpoch, CaptureCoordinatorError>> {
        None
    }
    fn resolve_configuration_epoch(
        &mut self,
        _epoch_id: u64,
        _resolution: ConfigurationEpochResolution,
    ) -> Result<(), CaptureCoordinatorError> {
        Err(CaptureCoordinatorError::policy(
            "live configuration epochs are unavailable on this platform",
        ))
    }
    fn take_configuration_epoch_notice(&mut self) -> Option<Result<(), CaptureCoordinatorError>> {
        None
    }
    fn replay_source_node(&self) -> Option<NodeId>;
    fn create_replay_attachment(
        &self,
    ) -> Result<Option<CaptureReplayAttachment>, CaptureCoordinatorError>;
    /// Remains true through Error cleanup until the supervisor has returned.
    fn is_active(&self) -> bool;

    fn graph_editing_enabled(&self) -> bool {
        !self.is_active()
    }
}

#[cfg(test)]
mod tests {
    use signal_capture::CaptureChannelId;
    use signal_capture_session::CaptureDataDelivery;

    use super::*;

    struct FixedDiscovery(CaptureAvailability);

    impl CaptureFeatureDiscovery for FixedDiscovery {
        fn discover_capture_availability(&self, _graph: &GraphState) -> CaptureAvailability {
            self.0.clone()
        }
    }

    #[test]
    fn discovered_live_capture_is_available_for_raw_capture() {
        let capabilities = CaptureProviderCapabilities::single(
            CaptureDataDelivery::DuringAcquisition,
            vec![CaptureChannelId::new("ui-test:0")],
            1_000_000,
        );
        let discovery = FixedDiscovery(CaptureAvailability::Available {
            source_node: NodeId(7),
            source_title: "UI Test Source".into(),
            has_trigger_program: false,
            capabilities,
            session_plan: None,
        });

        assert!(matches!(
            capture_availability(&GraphState::default(), &discovery, None),
            CaptureAvailability::Available { .. }
        ));
    }

    #[test]
    fn unavailable_discovery_reason_is_preserved() {
        let discovery = FixedDiscovery(CaptureAvailability::Unavailable {
            reason: "not a live source".into(),
        });

        let availability = capture_availability(&GraphState::default(), &discovery, None);
        assert_eq!(availability.reason(), Some("not a live source"));
    }
}
