use thiserror::Error;

use logic_analyzer_trigger::{TriggerIdentifier, TriggerValidationErrors};

/// Failure while applying a provider-neutral trigger-editor action.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TriggerEditorError {
    /// The current or edited program violates the provider schema.
    #[error("{0}")]
    Validation(
        #[from]
        #[source]
        TriggerValidationErrors,
    ),
    /// The schema's stage limit has been reached.
    #[error("this trigger schema supports at most {maximum} stage(s)")]
    StageLimit { maximum: usize },
    /// The requested stage index does not exist.
    #[error("trigger stage {stage} does not exist")]
    UnknownStage { stage: usize },
    /// The schema's per-stage predicate limit has been reached.
    #[error("this trigger schema supports at most {maximum} predicate(s) per stage")]
    PredicateLimit { maximum: usize },
    /// The requested predicate index does not exist.
    #[error("trigger predicate {stage}:{predicate} does not exist")]
    UnknownPredicate { stage: usize, predicate: usize },
    /// The requested registered predicate is absent from the schema.
    #[error("registered trigger predicate '{predicate}' is unknown")]
    UnknownRegisteredPredicate { predicate: TriggerIdentifier },
    /// A digital edit targeted a different predicate kind.
    #[error("the selected predicate is not a digital condition")]
    ExpectedDigitalPredicate,
    /// A registered-operand edit targeted a different predicate kind.
    #[error("the selected predicate is not registered")]
    ExpectedRegisteredPredicate,
    /// The requested operand is absent from the selected registered predicate.
    #[error("registered trigger operand '{operand}' is unknown")]
    UnknownRegisteredOperand { operand: TriggerIdentifier },
    /// The schema cannot supply a predicate for a new stage.
    #[error("this trigger schema has no predicate available for a new stage")]
    NoPredicateAvailable,
    /// The schema cannot supply a logic operator for a new stage.
    #[error("this trigger schema has no stage logic")]
    NoStageLogic,
    /// A channel operand has neither a valid default nor an enabled channel.
    #[error("a channel operand requires an enabled channel")]
    ChannelOperandWithoutChannel,
}
