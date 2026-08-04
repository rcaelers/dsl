use logic_analyzer_graph_plan::ProcessingGraphError;

/// Error produced while reconciling an active graph run.
#[derive(Debug)]
pub enum ApplyError {
    /// The edited graph did not lower; the active run is untouched.
    Compile(Vec<ProcessingGraphError>),
    /// The edit requires stopping and starting a new run.
    NeedsFullRestart(String),
    /// Runtime reconciliation failed after it began.
    Apply(String),
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
        }
    }
}

impl std::error::Error for ApplyError {}
