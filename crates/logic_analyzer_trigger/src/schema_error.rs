use thiserror::Error;

/// Failure while constructing a provider-neutral trigger schema.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TriggerSchemaError {
    /// A stable identifier was empty.
    #[error("a trigger identifier must not be empty")]
    EmptyIdentifier,
    /// A stable identifier contained unsupported characters.
    #[error(
        "trigger identifier '{value}' may contain only ASCII letters, digits, '.', '_', and '-'"
    )]
    InvalidIdentifierCharacters { value: String },
    /// Count capabilities advertised no counting mode.
    #[error("trigger count capabilities require at least one mode")]
    EmptyCountModes,
    /// Count capabilities repeated a counting mode.
    #[error("trigger count modes must be unique")]
    DuplicateCountModes,
    /// Count bounds or their step do not form a valid range.
    #[error("trigger count range or step is invalid")]
    InvalidCountRange,
    /// A choice has no user-facing label.
    #[error("a trigger choice label must not be empty")]
    EmptyChoiceLabel,
    /// An operand has no user-facing label.
    #[error("a trigger operand label must not be empty")]
    EmptyOperandLabel,
    /// An unsigned operand has inconsistent bounds, step, or default.
    #[error("trigger unsigned operand range, step, or default is invalid")]
    InvalidUnsignedOperandRange,
    /// A signed operand has inconsistent bounds, step, or default.
    #[error("trigger signed operand range, step, or default is invalid")]
    InvalidSignedOperandRange,
    /// A signed operand default does not lie on its configured step.
    #[error("trigger signed operand default is not on its configured step")]
    InvalidSignedOperandStep,
    /// A choice operand advertises no choices.
    #[error("a trigger choice operand requires at least one choice")]
    EmptyOperandChoices,
    /// A choice operand repeats a stable choice identity.
    #[error("trigger operand choice IDs must be unique")]
    DuplicateOperandChoices,
    /// A choice operand's default is absent from its choices.
    #[error("trigger operand default choice is not registered")]
    UnknownDefaultChoice,
    /// A byte operand has inconsistent bounds or default contents.
    #[error("trigger byte operand bounds or default are invalid")]
    InvalidByteOperand,
    /// A registered predicate has no user-facing label.
    #[error("a trigger predicate label must not be empty")]
    EmptyPredicateLabel,
    /// A registered predicate repeats an operand identity.
    #[error("trigger predicate operand IDs must be unique")]
    DuplicatePredicateOperands,
    /// A schema uses the reserved zero revision.
    #[error("trigger schema revision must be non-zero")]
    ZeroSchemaRevision,
    /// A schema permits no stages or no predicates per stage.
    #[error("trigger schema limits must be non-zero")]
    ZeroSchemaLimits,
    /// A schema advertises no stage logic operator.
    #[error("trigger schema logic operators must not be empty")]
    EmptyLogicOperators,
    /// A schema repeats a stage logic operator.
    #[error("trigger schema logic operators must be unique")]
    DuplicateLogicOperators,
    /// A schema explicitly advertises the condition represented by omission.
    #[error("Ignore is represented by an omitted digital predicate")]
    ExplicitIgnoreCondition,
    /// A schema repeats a digital condition.
    #[error("trigger digital conditions must be unique")]
    DuplicateDigitalConditions,
    /// A schema repeats a registered predicate identity.
    #[error("registered trigger predicate IDs must be unique")]
    DuplicateRegisteredPredicates,
    /// A schema cannot express the standard simple-trigger form.
    #[error("this trigger schema cannot represent an AND-combined simple trigger")]
    UnsupportedSimpleProgram,
}
