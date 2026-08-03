use serde::{Deserialize, Serialize};

use crate::WorkerOperation;

/// Opaque request for preparing a capture index in a host-owned execution context.
///
/// The operation owner defines the payload. Generic discovery and compiler
/// infrastructure only transports the request to the injected preparation
/// executor and never branches on its identifier or contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureIndexPreparationRequest {
    operation: WorkerOperation,
    payload: Vec<u8>,
}

impl CaptureIndexPreparationRequest {
    /// Creates an opaque host-executable capture-index preparation request.
    ///
    /// # Parameters
    /// - `operation`: Stable worker operation that owns the request codec.
    /// - `payload`: Operation-defined owned input bytes.
    pub fn new(operation: WorkerOperation, payload: Vec<u8>) -> Self {
        Self { operation, payload }
    }

    /// Returns the stable worker operation that interprets the payload.
    pub fn operation(&self) -> &WorkerOperation {
        &self.operation
    }

    /// Returns the operation-defined owned request bytes.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Consumes the request and returns its operation identifier and payload.
    pub fn into_parts(self) -> (WorkerOperation, Vec<u8>) {
        (self.operation, self.payload)
    }
}
