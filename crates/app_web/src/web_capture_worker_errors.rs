use std::fmt;

use logic_analyzer_graph_orchestration::GraphWorkerClientError;
use signal_capture::CaptureWorkerClientError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserCaptureWorkerInstallStage {
    BootstrapBlob,
    BootstrapUrl,
    WorkerStart,
    BootstrapUrlCleanup,
    WorkerInitialization,
    PumpStart,
}

impl fmt::Display for BrowserCaptureWorkerInstallStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BootstrapBlob => "create capture-worker bootstrap",
            Self::BootstrapUrl => "create capture-worker bootstrap URL",
            Self::WorkerStart => "create capture worker",
            Self::BootstrapUrlCleanup => "release capture-worker bootstrap URL",
            Self::WorkerInitialization => "initialize capture worker",
            Self::PumpStart => "start capture-worker pump",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BrowserCaptureWorkerInstallError {
    #[error("could not configure capture-worker client: {0}")]
    CaptureClient(#[source] CaptureWorkerClientError),
    #[error("could not configure graph-worker client: {0}")]
    GraphClient(#[source] GraphWorkerClientError),
    #[error("browser window is unavailable")]
    WindowUnavailable,
    #[error("could not construct capture-worker initialization message: {0}")]
    Message(#[source] BrowserWorkerMessageError),
    #[error("could not {stage}: {detail}")]
    Host {
        stage: BrowserCaptureWorkerInstallStage,
        detail: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BrowserCaptureAttachmentError {
    #[error("capture-worker attachment message is invalid: {0}")]
    Message(#[from] BrowserWorkerMessageError),
    #[error("could not attach browser file to capture worker: {detail}")]
    Submission { detail: String },
    #[error("capture worker returned an invalid content identity")]
    InvalidIdentity,
    #[error("capture worker returned invalid metadata: {0}")]
    Metadata(#[source] serde_json::Error),
    #[error("browser capture import failed: {message}")]
    Worker { message: String },
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum BrowserWorkerMessageError {
    #[error("worker message has no '{property}' property: {detail}")]
    PropertyAccess { property: String, detail: String },
    #[error("could not set worker message '{property}': {detail}")]
    PropertyWrite { property: String, detail: String },
    #[error("worker message '{property}' is not {expected}")]
    InvalidProperty {
        property: String,
        expected: &'static str,
    },
}

#[cfg(test)]
mod web_capture_worker_error_tests {
    use std::error::Error;
    use std::sync::Arc;

    use logic_analyzer_graph_orchestration::GraphWorkerClient;
    use platform_artifacts::MemoryArtifactRepository;
    use signal_capture::{CaptureMetadata, CaptureWorkerClient};

    use super::{
        BrowserCaptureAttachmentError, BrowserCaptureWorkerInstallError,
        BrowserCaptureWorkerInstallStage, BrowserWorkerMessageError,
    };

    #[test]
    fn installation_retains_capture_client_configuration_causes() {
        let cause = CaptureWorkerClient::new(0).err().unwrap();
        let error = BrowserCaptureWorkerInstallError::CaptureClient(cause);

        assert!(matches!(
            error,
            BrowserCaptureWorkerInstallError::CaptureClient(_)
        ));
        assert!(error.source().is_some());
    }

    #[test]
    fn attachment_retains_metadata_codec_causes() {
        let cause = serde_json::from_slice::<CaptureMetadata>(b"not metadata").unwrap_err();
        let error = BrowserCaptureAttachmentError::Metadata(cause);

        assert!(matches!(error, BrowserCaptureAttachmentError::Metadata(_)));
        assert!(error.source().is_some());
    }

    #[test]
    fn lifecycle_categories_have_stable_diagnostics() {
        for stage in [
            BrowserCaptureWorkerInstallStage::BootstrapBlob,
            BrowserCaptureWorkerInstallStage::BootstrapUrl,
            BrowserCaptureWorkerInstallStage::WorkerStart,
            BrowserCaptureWorkerInstallStage::BootstrapUrlCleanup,
            BrowserCaptureWorkerInstallStage::WorkerInitialization,
            BrowserCaptureWorkerInstallStage::PumpStart,
        ] {
            assert!(!stage.to_string().is_empty());
        }

        let graph_cause = GraphWorkerClient::new(0, Arc::new(MemoryArtifactRepository::new()))
            .err()
            .unwrap();
        let message_errors = [
            BrowserWorkerMessageError::PropertyAccess {
                property: "payload".to_owned(),
                detail: "unavailable".to_owned(),
            },
            BrowserWorkerMessageError::PropertyWrite {
                property: "payload".to_owned(),
                detail: "read-only".to_owned(),
            },
            BrowserWorkerMessageError::InvalidProperty {
                property: "sequence".to_owned(),
                expected: "an unsigned integer",
            },
        ];
        let errors = [
            BrowserCaptureWorkerInstallError::GraphClient(graph_cause).to_string(),
            BrowserCaptureWorkerInstallError::WindowUnavailable.to_string(),
            BrowserCaptureWorkerInstallError::Message(message_errors[0].clone()).to_string(),
            BrowserCaptureWorkerInstallError::Host {
                stage: BrowserCaptureWorkerInstallStage::WorkerStart,
                detail: "blocked".to_owned(),
            }
            .to_string(),
            BrowserCaptureAttachmentError::Submission {
                detail: "blocked".to_owned(),
            }
            .to_string(),
            BrowserCaptureAttachmentError::InvalidIdentity.to_string(),
            BrowserCaptureAttachmentError::Worker {
                message: "rejected".to_owned(),
            }
            .to_string(),
            message_errors[1].to_string(),
            message_errors[2].to_string(),
        ];

        assert!(errors.iter().all(|error| !error.is_empty()));
    }
}
