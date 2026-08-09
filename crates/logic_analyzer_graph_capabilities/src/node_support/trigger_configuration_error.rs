use thiserror::Error;

use logic_analyzer_trigger::TriggerValidationErrors;

/// Failure while assembling a graph node's neutral trigger configuration.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TriggerConfigurationError {
    /// More than one channel exposes the same provider identity.
    #[error("trigger configuration channel identities must be unique")]
    DuplicateChannelIdentities,
    /// More than one channel maps to the same viewer channel.
    #[error("trigger configuration viewer channels must be unique")]
    DuplicateViewerChannels,
    /// The current program violates the supplied trigger schema.
    #[error("invalid trigger program: {0}")]
    Program(
        #[from]
        #[source]
        TriggerValidationErrors,
    ),
}

#[cfg(test)]
mod trigger_configuration_error_tests {
    use std::error::Error;

    use logic_analyzer_trigger::TriggerValidationErrors;

    use super::TriggerConfigurationError;

    #[test]
    fn program_validation_cause_remains_available() {
        let error = TriggerConfigurationError::from(TriggerValidationErrors::schema_unavailable());

        assert!(matches!(&error, TriggerConfigurationError::Program(_)));
        assert!(error.source().is_some());
    }
}
