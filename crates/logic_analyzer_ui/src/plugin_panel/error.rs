//! Errors owned by the plugin-panel facade.

use std::error::Error as StdError;

use thiserror::Error;

/// Failure reported by a plugin while restoring its persisted panel state.
#[derive(Debug, Error)]
#[error("{source}")]
pub struct PluginPanelStateError {
    #[source]
    source: Box<dyn StdError + Send + Sync>,
}

impl PluginPanelStateError {
    /// Retains a typed plugin-owned cause as a panel-state failure.
    pub fn new(error: impl StdError + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }

    /// Creates a panel-state failure for providers that expose only a diagnostic.
    pub fn message(message: impl Into<String>) -> Self {
        Self::new(PluginPanelDiagnostic(message.into()))
    }
}

#[derive(Debug, Error)]
#[error("{0}")]
struct PluginPanelDiagnostic(String);

/// Failure to add a plugin-panel definition to the application registry.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PluginPanelRegistrationError {
    /// The persistable panel-kind identifier was empty.
    #[error("plugin panel identifiers must not be empty")]
    EmptyStableId,
    /// The user-facing title was empty.
    #[error("plugin panel '{stable_id}' must have a title")]
    EmptyTitle {
        /// Persistable identity of the invalid panel definition.
        stable_id: String,
    },
    /// Another panel kind already owns the same persistable identity.
    #[error("plugin panel '{stable_id}' is already registered")]
    DuplicateStableId {
        /// Persistable identity shared by both registrations.
        stable_id: String,
    },
}

pub(crate) fn validate_plugin_panel_definition(
    stable_id: &str,
    title: &str,
) -> Result<(), PluginPanelRegistrationError> {
    if stable_id.trim().is_empty() {
        return Err(PluginPanelRegistrationError::EmptyStableId);
    }
    if title.trim().is_empty() {
        return Err(PluginPanelRegistrationError::EmptyTitle {
            stable_id: stable_id.to_owned(),
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
#[error("Could not restore saved state for plugin panel '{title}': {source}")]
pub(crate) struct PluginPanelRestoreError {
    title: String,
    #[source]
    source: PluginPanelStateError,
}

impl PluginPanelRestoreError {
    pub(crate) fn new(title: impl Into<String>, source: PluginPanelStateError) -> Self {
        Self {
            title: title.into(),
            source,
        }
    }
}

#[cfg(test)]
mod error_tests {
    use std::error::Error as StdError;
    use std::fmt;

    use super::PluginPanelStateError;

    #[derive(Debug)]
    struct VersionMismatch;

    impl fmt::Display for VersionMismatch {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("unsupported state version")
        }
    }

    impl StdError for VersionMismatch {}

    #[test]
    fn state_error_retains_the_plugin_owned_cause() {
        let error = PluginPanelStateError::new(VersionMismatch);

        assert!(
            StdError::source(&error)
                .and_then(|source| source.downcast_ref::<VersionMismatch>())
                .is_some()
        );
    }
}
