//! Portable logic-analyzer trigger programs, capability schemas, and validation.
//!
//! This crate owns the serializable trigger domain shared by capture providers, graph features,
//! editors, viewers, compilers, and application composition. It depends only on the opaque capture
//! channel identity and contains no device transport, acquisition lifecycle, graph, or UI behavior.

#[cfg(test)]
mod architecture_tests;
mod condition;
mod program;
mod schema_error;

pub use condition::SimpleTriggerCondition;
pub use program::{
    RegisteredTriggerPredicateSchema, TRIGGER_PROGRAM_FORMAT_VERSION, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate, TriggerProgram, TriggerProgramEditError, TriggerProgramForm, TriggerStage,
    TriggerValidationCode, TriggerValidationDiagnostic, TriggerValidationErrors,
    ValidatedTriggerProgram,
};
pub use schema_error::TriggerSchemaError;
