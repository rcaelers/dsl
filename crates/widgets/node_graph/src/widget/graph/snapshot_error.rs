use thiserror::Error;

/// Failure while synchronizing and serializing an editor graph snapshot.
#[derive(Debug, Error)]
#[error("could not serialize graph snapshot: {source}")]
pub struct GraphSnapshotError {
    /// Concrete JSON serialization failure.
    #[source]
    source: serde_json::Error,
}

impl From<serde_json::Error> for GraphSnapshotError {
    fn from(source: serde_json::Error) -> Self {
        Self { source }
    }
}

#[cfg(test)]
mod snapshot_error_tests {
    use std::error::Error;

    use serde::Serialize;
    use serde::ser::{Error as _, Serializer};

    use super::GraphSnapshotError;

    struct FailingValue;

    impl Serialize for FailingValue {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            Err(S::Error::custom("controlled serialization failure"))
        }
    }

    #[test]
    fn snapshot_error_retains_the_json_source() {
        let source = serde_json::to_value(FailingValue).unwrap_err();
        let error = GraphSnapshotError::from(source);

        assert_eq!(
            error.source().map(ToString::to_string).as_deref(),
            Some("controlled serialization failure")
        );
    }
}
