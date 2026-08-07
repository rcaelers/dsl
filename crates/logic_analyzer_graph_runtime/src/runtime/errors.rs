use logic_analyzer_graph_plan::ProcessingGraphError;
use signal_runtime::PipelineError;

/// Error produced while reconciling an active graph run.
#[derive(Debug)]
pub enum ApplyError {
    /// The edited graph did not lower; the active run is untouched.
    Compile(Vec<ProcessingGraphError>),
    /// The edit requires stopping and starting a new run.
    NeedsFullRestart(String),
    /// Runtime reconciliation failed after it began.
    Apply(String),
    /// Stream-pipeline supervision rejected a live reconciliation operation.
    Runtime(PipelineError),
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compile(errors) => {
                write!(
                    formatter,
                    "edited graph has {} compile error(s)",
                    errors.len()
                )
            }
            Self::NeedsFullRestart(message) => {
                write!(formatter, "live edit requires a full restart: {message}")
            }
            Self::Apply(message) => write!(formatter, "could not apply live edit: {message}"),
            Self::Runtime(error) => write!(formatter, "could not apply live edit: {error}"),
        }
    }
}

impl std::error::Error for ApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::Compile(_) | Self::NeedsFullRestart(_) | Self::Apply(_) => None,
        }
    }
}
