use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Failure to translate between a graph document value and node-owned persisted state.
#[derive(Clone, Debug, thiserror::Error)]
pub enum PersistedStateError {
    /// The generic document value could not be decoded as the node's state record.
    #[error("invalid node state: {0}")]
    Decode(#[source] Arc<serde_json::Error>),
    /// An edited node state record could not be encoded as a generic document value.
    #[error("could not encode node state: {0}")]
    Encode(#[source] Arc<serde_json::Error>),
}

impl PartialEq for PersistedStateError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Decode(left), Self::Decode(right))
            | (Self::Encode(left), Self::Encode(right)) => left.to_string() == right.to_string(),
            _ => false,
        }
    }
}

impl Eq for PersistedStateError {}

/// Deserializes node-owned persisted state from the generic graph document.
///
/// Concrete features call this at their load boundary. The typed error retains
/// the JSON codec cause for the feature and its consumer to classify.
pub fn parse_state<T: DeserializeOwned>(state: &Value) -> Result<T, PersistedStateError> {
    serde_json::from_value(state.clone())
        .map_err(Arc::new)
        .map_err(PersistedStateError::Decode)
}

/// Serializes edited node-owned state into the generic graph document value.
pub fn serialize_state<T: Serialize>(state: T) -> Result<Value, PersistedStateError> {
    serde_json::to_value(state)
        .map_err(Arc::new)
        .map_err(PersistedStateError::Encode)
}

#[cfg(test)]
mod state_tests {
    use std::error::Error as _;

    use serde::Deserialize;
    use serde_json::json;

    use super::{PersistedStateError, parse_state};

    #[derive(Debug, Deserialize)]
    struct RequiredState {
        _value: usize,
    }

    #[test]
    fn decoding_retains_the_json_codec_cause() {
        let error = parse_state::<RequiredState>(&json!({})).expect_err("state must be invalid");

        assert!(matches!(error, PersistedStateError::Decode(_)));
        assert!(error.source().is_some());
    }
}
