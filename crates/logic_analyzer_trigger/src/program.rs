//! Provider-neutral advanced-trigger schemas, programs, and validation.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use signal_capture::CaptureChannelId;

use super::condition::SimpleTriggerCondition;
use super::schema_error::TriggerSchemaError;

/// Serialized format version for [`TriggerProgram`] values.
pub const TRIGGER_PROGRAM_FORMAT_VERSION: u16 = 1;

/// Stable registered identity used by schemas, predicates, operands, and choices.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TriggerIdentifier(String);

impl TriggerIdentifier {
    /// Validates and constructs a stable trigger identifier.
    ///
    /// Identifiers are non-empty ASCII strings containing letters, digits, `.`,
    /// `_`, or `-`; they are persisted in trigger schemas and programs.
    ///
    /// # Parameters
    /// - `value`: Input consumed by this operation.
    pub fn new(value: impl Into<String>) -> Result<Self, TriggerSchemaError> {
        let value = value.into();
        if value.is_empty() {
            return Err(TriggerSchemaError::EmptyIdentifier);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(TriggerSchemaError::InvalidIdentifierCharacters { value });
        }
        Ok(Self(value))
    }

    /// Returns the validated identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TriggerIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TriggerIdentifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Boolean operator used to combine predicates in a trigger stage.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerLogicOperator {
    /// Stage fires when every predicate matches.
    And,
    /// Stage fires when at least one predicate matches.
    Or,
    /// Stage fires when an odd number of predicates match.
    Xor,
    /// Negated conjunction of all predicates.
    Nand,
    /// Negated disjunction of all predicates.
    Nor,
}

/// Counting interpretation supported by a trigger provider.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerCountMode {
    /// Count matching predicate occurrences, allowing non-matches between them.
    Occurrences,
    /// Count only adjacent matching predicate occurrences.
    Consecutive,
}

/// Provider-advertised valid modes and range for a trigger-count operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerCountCapabilities {
    modes: Vec<TriggerCountMode>,
    minimum: u64,
    maximum: u64,
    step: u64,
}

impl TriggerCountCapabilities {
    /// Creates validated count capabilities.
    ///
    /// The mode list must be non-empty and unique, and `step` must describe a
    /// non-empty inclusive range from `minimum` through `maximum`.
    pub fn new(
        modes: Vec<TriggerCountMode>,
        minimum: u64,
        maximum: u64,
        step: u64,
    ) -> Result<Self, TriggerSchemaError> {
        if modes.is_empty() {
            return Err(TriggerSchemaError::EmptyCountModes);
        }
        if modes.iter().copied().collect::<HashSet<_>>().len() != modes.len() {
            return Err(TriggerSchemaError::DuplicateCountModes);
        }
        if minimum > maximum || step == 0 {
            return Err(TriggerSchemaError::InvalidCountRange);
        }
        Ok(Self {
            modes,
            minimum,
            maximum,
            step,
        })
    }

    /// Returns the supported counting modes.
    pub fn modes(&self) -> &[TriggerCountMode] {
        &self.modes
    }

    /// Returns the inclusive lower count bound.
    pub const fn minimum(&self) -> u64 {
        self.minimum
    }

    /// Returns the inclusive upper count bound.
    pub const fn maximum(&self) -> u64 {
        self.maximum
    }

    /// Returns the increment between supported count values.
    pub const fn step(&self) -> u64 {
        self.step
    }
}

/// One stable value offered by a choice trigger operand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerChoice {
    id: TriggerIdentifier,
    label: String,
}

impl TriggerChoice {
    /// Creates a choice with a stable persisted ID and a non-empty display label.
    ///
    /// # Parameters
    /// - `id`: Stable value persisted in trigger programs.
    /// - `label`: Non-empty provider-facing display label.
    pub fn new(
        id: TriggerIdentifier,
        label: impl Into<String>,
    ) -> Result<Self, TriggerSchemaError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TriggerSchemaError::EmptyChoiceLabel);
        }
        Ok(Self { id, label })
    }

    /// Returns the stable value persisted in trigger programs.
    pub fn id(&self) -> &TriggerIdentifier {
        &self.id
    }

    /// Returns the provider-facing display label.
    pub fn label(&self) -> &str {
        &self.label
    }
}

/// The editable value domain of a registered trigger predicate operand.
///
/// The schema carries provider capabilities only; it does not define the runtime
/// semantics of a concrete predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerOperandKind {
    /// Boolean operand with a default value.
    Boolean {
        /// Value selected when the predicate is first created.
        default: bool,
    },
    /// Unsigned numeric operand with an inclusive stepped range.
    Unsigned {
        /// Inclusive lower bound.
        minimum: u64,
        /// Inclusive upper bound.
        maximum: u64,
        /// Allowed increment.
        step: u64,
        /// Value selected when the predicate is first created.
        default: u64,
    },
    /// Signed numeric operand with an inclusive stepped range.
    Signed {
        /// Inclusive lower bound.
        minimum: i64,
        /// Inclusive upper bound.
        maximum: i64,
        /// Allowed increment.
        step: u64,
        /// Value selected when the predicate is first created.
        default: i64,
    },
    /// Nanosecond duration operand with an inclusive stepped range.
    DurationNs {
        /// Inclusive lower duration.
        minimum: u64,
        /// Inclusive upper duration.
        maximum: u64,
        /// Allowed duration increment.
        step: u64,
        /// Default duration.
        default: u64,
    },
    /// Operand selected from a provider-defined stable choice list.
    Choice {
        /// Available stable choices and labels.
        choices: Vec<TriggerChoice>,
        /// Choice selected when the predicate is first created.
        default: TriggerIdentifier,
    },
    /// Operand that selects an enabled capture channel.
    Channel {
        /// Initial selected channel, or no selection.
        default: Option<CaptureChannelId>,
    },
    /// Byte-string operand with length constraints.
    Bytes {
        /// Minimum accepted byte count.
        minimum_length: usize,
        /// Maximum accepted byte count.
        maximum_length: usize,
        /// Default byte string.
        default: Vec<u8>,
    },
}

impl TriggerOperandKind {
    fn validate_schema(&self) -> Result<(), TriggerSchemaError> {
        match self {
            Self::Boolean { .. } | Self::Channel { .. } => Ok(()),
            Self::Unsigned {
                minimum,
                maximum,
                step,
                default,
            }
            | Self::DurationNs {
                minimum,
                maximum,
                step,
                default,
            } => validate_unsigned_range(*minimum, *maximum, *step, *default),
            Self::Signed {
                minimum,
                maximum,
                step,
                default,
            } => validate_signed_range(*minimum, *maximum, *step, *default),
            Self::Choice { choices, default } => {
                if choices.is_empty() {
                    return Err(TriggerSchemaError::EmptyOperandChoices);
                }
                let ids: HashSet<_> = choices.iter().map(|choice| &choice.id).collect();
                if ids.len() != choices.len() {
                    return Err(TriggerSchemaError::DuplicateOperandChoices);
                }
                if !ids.contains(default) {
                    return Err(TriggerSchemaError::UnknownDefaultChoice);
                }
                Ok(())
            }
            Self::Bytes {
                minimum_length,
                maximum_length,
                default,
            } => {
                if minimum_length > maximum_length
                    || default.len() < *minimum_length
                    || default.len() > *maximum_length
                {
                    return Err(TriggerSchemaError::InvalidByteOperand);
                }
                Ok(())
            }
        }
    }
}

/// Validated schema for one operand of a registered trigger predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerOperandSchema {
    id: TriggerIdentifier,
    label: String,
    kind: TriggerOperandKind,
}

impl TriggerOperandSchema {
    /// Creates a validated operand definition for a registered predicate.
    ///
    /// # Parameters
    /// - `id`: Stable operand identity persisted in trigger programs.
    /// - `label`: Non-empty label for schema-driven editors.
    /// - `kind`: Supported value domain and its default.
    pub fn new(
        id: TriggerIdentifier,
        label: impl Into<String>,
        kind: TriggerOperandKind,
    ) -> Result<Self, TriggerSchemaError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TriggerSchemaError::EmptyOperandLabel);
        }
        kind.validate_schema()?;
        Ok(Self { id, label, kind })
    }

    /// Returns the operand's stable identifier.
    pub fn id(&self) -> &TriggerIdentifier {
        &self.id
    }

    /// Returns the operand label shown by schema-driven editors.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the operand's value domain and default.
    pub fn kind(&self) -> &TriggerOperandKind {
        &self.kind
    }
}

/// Provider-registered trigger predicate and its operand schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredTriggerPredicateSchema {
    id: TriggerIdentifier,
    label: String,
    operands: Vec<TriggerOperandSchema>,
}

impl RegisteredTriggerPredicateSchema {
    /// Creates a predicate schema with unique, validated operand identifiers.
    ///
    /// # Parameters
    /// - `id`: Stable provider predicate identity.
    /// - `label`: Non-empty label for schema-driven editors.
    /// - `operands`: Ordered operand schemas with unique identifiers.
    pub fn new(
        id: TriggerIdentifier,
        label: impl Into<String>,
        operands: Vec<TriggerOperandSchema>,
    ) -> Result<Self, TriggerSchemaError> {
        let label = label.into();
        if label.trim().is_empty() {
            return Err(TriggerSchemaError::EmptyPredicateLabel);
        }
        if operands
            .iter()
            .map(|operand| &operand.id)
            .collect::<HashSet<_>>()
            .len()
            != operands.len()
        {
            return Err(TriggerSchemaError::DuplicatePredicateOperands);
        }
        Ok(Self {
            id,
            label,
            operands,
        })
    }

    /// Returns the predicate's stable identifier.
    pub fn id(&self) -> &TriggerIdentifier {
        &self.id
    }

    /// Returns the predicate label displayed by a trigger editor.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the predicate operands in display and serialization order.
    pub fn operands(&self) -> &[TriggerOperandSchema] {
        &self.operands
    }
}

/// Provider capabilities and registered predicates for an advanced trigger editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerEditorSchema {
    id: TriggerIdentifier,
    revision: u32,
    maximum_stages: usize,
    maximum_predicates_per_stage: usize,
    logic_operators: Vec<TriggerLogicOperator>,
    digital_conditions: Vec<SimpleTriggerCondition>,
    stage_inversion: bool,
    count: Option<TriggerCountCapabilities>,
    predicates: Vec<RegisteredTriggerPredicateSchema>,
}

impl TriggerEditorSchema {
    /// Creates a provider-neutral editor schema with validated stage limits and operators.
    ///
    /// # Parameters
    /// - `id`: Stable provider schema identity.
    /// - `revision`: Non-zero revision used to detect incompatible saved programs.
    /// - `maximum_stages`: Maximum number of stages supported by the provider.
    /// - `maximum_predicates_per_stage`: Maximum predicates permitted in one stage.
    /// - `logic_operators`: Non-empty unique set of supported boolean operators.
    pub fn new(
        id: TriggerIdentifier,
        revision: u32,
        maximum_stages: usize,
        maximum_predicates_per_stage: usize,
        logic_operators: Vec<TriggerLogicOperator>,
    ) -> Result<Self, TriggerSchemaError> {
        if revision == 0 {
            return Err(TriggerSchemaError::ZeroSchemaRevision);
        }
        if maximum_stages == 0 || maximum_predicates_per_stage == 0 {
            return Err(TriggerSchemaError::ZeroSchemaLimits);
        }
        if logic_operators.is_empty() {
            return Err(TriggerSchemaError::EmptyLogicOperators);
        }
        if logic_operators
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .len()
            != logic_operators.len()
        {
            return Err(TriggerSchemaError::DuplicateLogicOperators);
        }
        Ok(Self {
            id,
            revision,
            maximum_stages,
            maximum_predicates_per_stage,
            logic_operators,
            digital_conditions: Vec::new(),
            stage_inversion: false,
            count: None,
            predicates: Vec::new(),
        })
    }

    /// Adds the supported simple digital conditions to this schema.
    pub fn with_digital_conditions(
        mut self,
        conditions: Vec<SimpleTriggerCondition>,
    ) -> Result<Self, TriggerSchemaError> {
        if conditions.contains(&SimpleTriggerCondition::Ignore) {
            return Err(TriggerSchemaError::ExplicitIgnoreCondition);
        }
        if conditions.iter().copied().collect::<HashSet<_>>().len() != conditions.len() {
            return Err(TriggerSchemaError::DuplicateDigitalConditions);
        }
        self.digital_conditions = conditions;
        Ok(self)
    }

    /// Sets whether stages may be inverted.
    pub const fn with_stage_inversion(mut self, enabled: bool) -> Self {
        self.stage_inversion = enabled;
        self
    }

    /// Adds a provider-advertised trigger-count capability.
    pub fn with_count(mut self, count: TriggerCountCapabilities) -> Self {
        self.count = Some(count);
        self
    }

    /// Adds validated provider-specific predicates to the editor schema.
    pub fn with_registered_predicates(
        mut self,
        predicates: Vec<RegisteredTriggerPredicateSchema>,
    ) -> Result<Self, TriggerSchemaError> {
        if predicates
            .iter()
            .map(|predicate| &predicate.id)
            .collect::<HashSet<_>>()
            .len()
            != predicates.len()
        {
            return Err(TriggerSchemaError::DuplicateRegisteredPredicates);
        }
        self.predicates = predicates;
        Ok(self)
    }

    /// Returns the schema's stable identifier.
    pub fn id(&self) -> &TriggerIdentifier {
        &self.id
    }

    /// Returns the provider-defined schema revision.
    pub const fn revision(&self) -> u32 {
        self.revision
    }

    /// Returns the maximum number of stages accepted by this schema.
    pub const fn maximum_stages(&self) -> usize {
        self.maximum_stages
    }

    /// Returns the maximum number of predicates allowed in one stage.
    pub const fn maximum_predicates_per_stage(&self) -> usize {
        self.maximum_predicates_per_stage
    }

    /// Returns the boolean operators available to stage predicates.
    pub fn logic_operators(&self) -> &[TriggerLogicOperator] {
        &self.logic_operators
    }

    /// Returns supported simple digital conditions.
    pub fn digital_conditions(&self) -> &[SimpleTriggerCondition] {
        &self.digital_conditions
    }

    /// Reports whether a stage may invert its combined predicate result.
    pub const fn supports_stage_inversion(&self) -> bool {
        self.stage_inversion
    }

    /// Returns count capabilities when the provider supports count predicates.
    pub fn count_capabilities(&self) -> Option<&TriggerCountCapabilities> {
        self.count.as_ref()
    }

    /// Returns provider-specific predicates registered with this schema.
    pub fn registered_predicates(&self) -> &[RegisteredTriggerPredicateSchema] {
        &self.predicates
    }

    /// Builds the equivalent single-stage digital program, or `None` for free run.
    ///
    /// # Parameters
    /// - `conditions`: Simple conditions keyed by capture channel; `Ignore` is omitted.
    pub fn simple_program(
        &self,
        conditions: impl IntoIterator<Item = (CaptureChannelId, SimpleTriggerCondition)>,
    ) -> Result<Option<TriggerProgram>, TriggerSchemaError> {
        if !self.logic_operators.contains(&TriggerLogicOperator::And) {
            return Err(TriggerSchemaError::UnsupportedSimpleProgram);
        }
        let predicates: Vec<_> = conditions
            .into_iter()
            .filter(|(_, condition)| *condition != SimpleTriggerCondition::Ignore)
            .map(|(channel, condition)| TriggerPredicate::Digital { channel, condition })
            .collect();
        Ok((!predicates.is_empty()).then(|| {
            TriggerProgram::new(
                self.id.clone(),
                self.revision,
                vec![TriggerStage {
                    predicates,
                    logic: TriggerLogicOperator::And,
                    inverted: false,
                    count: None,
                }],
            )
        }))
    }

    /// Classifies a validated program without interpreting registered predicate identities.
    ///
    /// The common digital form is the one representation lane trigger controls may edit safely:
    /// one non-inverted, uncounted AND stage containing at most one digital predicate per channel.
    pub fn program_form(
        &self,
        program: Option<&TriggerProgram>,
        channels: &[CaptureChannelId],
    ) -> Result<TriggerProgramForm, TriggerValidationErrors> {
        let Some(program) = program else {
            return Ok(TriggerProgramForm::FreeRun);
        };
        self.validate_program(program, channels)?;
        let [stage] = program.stages.as_slice() else {
            return Ok(TriggerProgramForm::Advanced);
        };
        if stage.logic != TriggerLogicOperator::And || stage.inverted || stage.count.is_some() {
            return Ok(TriggerProgramForm::Advanced);
        }
        let mut conditions = BTreeMap::new();
        for predicate in &stage.predicates {
            let TriggerPredicate::Digital { channel, condition } = predicate else {
                return Ok(TriggerProgramForm::Advanced);
            };
            if conditions.insert(channel.clone(), *condition).is_some() {
                return Ok(TriggerProgramForm::Advanced);
            }
        }
        Ok(TriggerProgramForm::CommonDigital(conditions))
    }

    /// Applies one lane condition without replacing an advanced program implicitly.
    pub fn with_simple_condition(
        &self,
        program: Option<&TriggerProgram>,
        channels: &[CaptureChannelId],
        channel: &CaptureChannelId,
        condition: SimpleTriggerCondition,
    ) -> Result<Option<TriggerProgram>, TriggerProgramEditError> {
        if !channels.contains(channel) {
            return Err(TriggerProgramEditError::UnknownChannel(channel.clone()));
        }
        let mut conditions = match self.program_form(program, channels)? {
            TriggerProgramForm::FreeRun => BTreeMap::new(),
            TriggerProgramForm::CommonDigital(conditions) => conditions,
            TriggerProgramForm::Advanced => return Err(TriggerProgramEditError::AdvancedProgram),
        };
        if condition == SimpleTriggerCondition::Ignore {
            conditions.remove(channel);
        } else {
            conditions.insert(channel.clone(), condition);
        }
        let program = self
            .simple_program(conditions)?
            .filter(|program| !program.stages.is_empty());
        if let Some(program) = &program {
            self.validate_program(program, channels)?;
        }
        Ok(program)
    }

    /// Validates a program against this schema and the enabled capture channels.
    ///
    /// Validation is semantic only: providers remain responsible for executing their
    /// registered predicate identities.
    ///
    /// # Parameters
    /// - `program`: Program to validate against this schema.
    /// - `channels`: Enabled capture channels accepted by digital predicates.
    pub fn validate_program(
        &self,
        program: &TriggerProgram,
        channels: &[CaptureChannelId],
    ) -> Result<ValidatedTriggerProgram, TriggerValidationErrors> {
        let mut diagnostics = Vec::new();
        if program.format_version != TRIGGER_PROGRAM_FORMAT_VERSION {
            diagnostics.push(diagnostic(
                "program.format_version",
                TriggerValidationCode::FormatVersion,
                format!(
                    "trigger program format {} is unsupported",
                    program.format_version
                ),
            ));
        }
        if program.schema_id != self.id {
            diagnostics.push(diagnostic(
                "program.schema_id",
                TriggerValidationCode::SchemaIdentity,
                format!(
                    "trigger program targets schema '{}', expected '{}'",
                    program.schema_id, self.id
                ),
            ));
        }
        if program.schema_revision != self.revision {
            diagnostics.push(diagnostic(
                "program.schema_revision",
                TriggerValidationCode::SchemaRevision,
                format!(
                    "trigger program targets schema revision {}, expected {}",
                    program.schema_revision, self.revision
                ),
            ));
        }
        if program.stages.is_empty() || program.stages.len() > self.maximum_stages {
            diagnostics.push(diagnostic(
                "program.stages",
                TriggerValidationCode::StageLimit,
                format!(
                    "trigger program requires 1..={} stages",
                    self.maximum_stages
                ),
            ));
        }
        let channel_set: HashSet<_> = channels.iter().collect();
        for (stage_index, stage) in program.stages.iter().enumerate() {
            let stage_path = format!("program.stages[{stage_index}]");
            let mut digital_channels = HashSet::new();
            if stage.predicates.is_empty()
                || stage.predicates.len() > self.maximum_predicates_per_stage
            {
                diagnostics.push(diagnostic(
                    format!("{stage_path}.predicates"),
                    TriggerValidationCode::PredicateLimit,
                    format!(
                        "trigger stage requires 1..={} predicates",
                        self.maximum_predicates_per_stage
                    ),
                ));
            }
            if !self.logic_operators.contains(&stage.logic) {
                diagnostics.push(diagnostic(
                    format!("{stage_path}.logic"),
                    TriggerValidationCode::UnsupportedLogic,
                    format!("trigger stage logic {:?} is unsupported", stage.logic),
                ));
            }
            if stage.inverted && !self.stage_inversion {
                diagnostics.push(diagnostic(
                    format!("{stage_path}.inverted"),
                    TriggerValidationCode::UnsupportedInversion,
                    "trigger stage inversion is unsupported",
                ));
            }
            self.validate_count(stage.count, &stage_path, &mut diagnostics);
            for (predicate_index, predicate) in stage.predicates.iter().enumerate() {
                let path = format!("{stage_path}.predicates[{predicate_index}]");
                if let TriggerPredicate::Digital { channel, .. } = predicate
                    && !digital_channels.insert(channel)
                {
                    diagnostics.push(diagnostic(
                        format!("{path}.channel"),
                        TriggerValidationCode::DuplicateChannel,
                        format!("capture channel '{channel}' appears more than once in this trigger stage"),
                    ));
                }
                self.validate_predicate(predicate, &path, &channel_set, &mut diagnostics);
            }
        }
        if diagnostics.is_empty() {
            Ok(ValidatedTriggerProgram(program.clone()))
        } else {
            Err(TriggerValidationErrors { diagnostics })
        }
    }

    fn validate_count(
        &self,
        count: Option<TriggerCount>,
        stage_path: &str,
        diagnostics: &mut Vec<TriggerValidationDiagnostic>,
    ) {
        let Some(count) = count else {
            return;
        };
        let Some(capabilities) = &self.count else {
            diagnostics.push(diagnostic(
                format!("{stage_path}.count"),
                TriggerValidationCode::UnsupportedCount,
                "trigger stage counting is unsupported",
            ));
            return;
        };
        if !capabilities.modes.contains(&count.mode) {
            diagnostics.push(diagnostic(
                format!("{stage_path}.count.mode"),
                TriggerValidationCode::UnsupportedCountMode,
                format!("trigger count mode {:?} is unsupported", count.mode),
            ));
        }
        if count.value < capabilities.minimum
            || count.value > capabilities.maximum
            || !(count.value - capabilities.minimum).is_multiple_of(capabilities.step)
        {
            diagnostics.push(diagnostic(
                format!("{stage_path}.count.value"),
                TriggerValidationCode::CountRange,
                format!(
                    "trigger count must be {}..={} in steps of {}",
                    capabilities.minimum, capabilities.maximum, capabilities.step
                ),
            ));
        }
    }

    fn validate_predicate(
        &self,
        predicate: &TriggerPredicate,
        path: &str,
        channels: &HashSet<&CaptureChannelId>,
        diagnostics: &mut Vec<TriggerValidationDiagnostic>,
    ) {
        match predicate {
            TriggerPredicate::Digital { channel, condition } => {
                if !channels.contains(channel) {
                    diagnostics.push(diagnostic(
                        format!("{path}.channel"),
                        TriggerValidationCode::UnknownChannel,
                        format!("capture channel '{channel}' is not enabled"),
                    ));
                }
                if *condition == SimpleTriggerCondition::Ignore
                    || !self.digital_conditions.contains(condition)
                {
                    diagnostics.push(diagnostic(
                        format!("{path}.condition"),
                        TriggerValidationCode::UnsupportedDigitalCondition,
                        format!("digital trigger condition {condition:?} is unsupported"),
                    ));
                }
            }
            TriggerPredicate::Registered {
                predicate,
                operands,
            } => {
                let Some(schema) = self
                    .predicates
                    .iter()
                    .find(|schema| schema.id == *predicate)
                else {
                    diagnostics.push(diagnostic(
                        format!("{path}.predicate"),
                        TriggerValidationCode::UnknownPredicate,
                        format!("registered trigger predicate '{predicate}' is unknown"),
                    ));
                    return;
                };
                for operand in &schema.operands {
                    match operands.get(&operand.id) {
                        Some(value) => validate_operand(
                            &operand.kind,
                            value,
                            &format!("{path}.operands.{}", operand.id),
                            channels,
                            diagnostics,
                        ),
                        None => diagnostics.push(diagnostic(
                            format!("{path}.operands.{}", operand.id),
                            TriggerValidationCode::MissingOperand,
                            format!("trigger operand '{}' is required", operand.id),
                        )),
                    }
                }
                for operand in operands.keys() {
                    if !schema.operands.iter().any(|schema| schema.id == *operand) {
                        diagnostics.push(diagnostic(
                            format!("{path}.operands.{operand}"),
                            TriggerValidationCode::UnexpectedOperand,
                            format!("trigger operand '{operand}' is not registered"),
                        ));
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TriggerProgram {
    /// Serialized trigger-program format version.
    pub format_version: u16,
    /// Stable identity of the schema that validates this program.
    pub schema_id: TriggerIdentifier,
    /// Revision of that schema used to create this program.
    pub schema_revision: u32,
    /// Ordered trigger stages evaluated by the provider.
    pub stages: Vec<TriggerStage>,
}

/// Simplified classification of a trigger program for common UI controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerProgramForm {
    /// No trigger; capture runs freely.
    FreeRun,
    /// Simple per-channel digital conditions.
    CommonDigital(BTreeMap<CaptureChannelId, SimpleTriggerCondition>),
    /// Program requires the advanced trigger editor.
    Advanced,
}

/// Failure while converting between common controls and a trigger program.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum TriggerProgramEditError {
    #[error("{0}")]
    Validation(
        #[from]
        #[source]
        TriggerValidationErrors,
    ),
    #[error("lane controls cannot replace an advanced trigger program; use the Triggers panel")]
    AdvancedProgram,
    #[error("capture channel '{0}' is not enabled")]
    UnknownChannel(CaptureChannelId),
    #[error("{0}")]
    Schema(
        #[from]
        #[source]
        TriggerSchemaError,
    ),
}

impl TriggerProgram {
    /// Creates a program for the supplied schema revision using the current format version.
    ///
    /// # Parameters
    /// - `schema_id`: Stable schema identity that validates this program.
    /// - `schema_revision`: Schema revision used to create the program.
    /// - `stages`: Ordered predicate stages evaluated by the provider.
    pub fn new(
        schema_id: TriggerIdentifier,
        schema_revision: u32,
        stages: Vec<TriggerStage>,
    ) -> Self {
        Self {
            format_version: TRIGGER_PROGRAM_FORMAT_VERSION,
            schema_id,
            schema_revision,
            stages,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TriggerStage {
    /// Predicates combined in this stage.
    pub predicates: Vec<TriggerPredicate>,
    /// Boolean operation used to combine predicates.
    pub logic: TriggerLogicOperator,
    /// Whether the combined result is negated.
    pub inverted: bool,
    /// Optional repetition requirement before the stage completes.
    pub count: Option<TriggerCount>,
}

/// Repetition requirement applied to one trigger stage.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct TriggerCount {
    /// Interpretation of the count.
    pub mode: TriggerCountMode,
    /// Number of matching occurrences required.
    pub value: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerPredicate {
    /// Built-in digital edge or level predicate.
    Digital {
        /// Capture channel to test.
        channel: CaptureChannelId,
        /// Required logic condition on the channel.
        condition: SimpleTriggerCondition,
    },
    /// Provider-registered predicate with typed operands.
    Registered {
        /// Stable provider predicate identity.
        predicate: TriggerIdentifier,
        /// Values keyed by stable operand identity.
        operands: BTreeMap<TriggerIdentifier, TriggerOperandValue>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TriggerOperandValue {
    /// Boolean value.
    Boolean(bool),
    /// Unsigned numeric value.
    Unsigned(u64),
    /// Signed numeric value.
    Signed(i64),
    /// Duration in nanoseconds.
    DurationNs(u64),
    /// Provider-defined stable choice identity.
    Choice(TriggerIdentifier),
    /// Enabled capture channel identity.
    Channel(CaptureChannelId),
    /// Arbitrary byte string.
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedTriggerProgram(TriggerProgram);

impl ValidatedTriggerProgram {
    /// Returns the program that was validated against its schema.
    pub fn program(&self) -> &TriggerProgram {
        &self.0
    }

    /// Consumes this validation result and returns the validated program.
    pub fn into_program(self) -> TriggerProgram {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TriggerValidationCode {
    SchemaUnavailable,
    FormatVersion,
    SchemaIdentity,
    SchemaRevision,
    StageLimit,
    PredicateLimit,
    UnsupportedLogic,
    UnsupportedInversion,
    UnsupportedCount,
    UnsupportedCountMode,
    CountRange,
    UnknownChannel,
    DuplicateChannel,
    UnsupportedDigitalCondition,
    UnknownPredicate,
    MissingOperand,
    UnexpectedOperand,
    OperandType,
    OperandRange,
    OperandStep,
    UnknownChoice,
    ByteLength,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerValidationDiagnostic {
    /// Structured path to the invalid program element.
    pub path: String,
    /// Machine-readable category of the validation failure.
    pub code: TriggerValidationCode,
    /// User-presentable explanation of the failure.
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerValidationErrors {
    diagnostics: Vec<TriggerValidationDiagnostic>,
}

impl fmt::Display for TriggerValidationErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.diagnostics.len() != 1 {
            write!(
                formatter,
                "trigger program has {} validation errors: ",
                self.diagnostics.len()
            )?;
        }
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index != 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{}: {}", diagnostic.path, diagnostic.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for TriggerValidationErrors {}

impl TriggerValidationErrors {
    /// Returns every diagnostic collected while validating the program.
    pub fn diagnostics(&self) -> &[TriggerValidationDiagnostic] {
        &self.diagnostics
    }

    /// Creates the diagnostic returned when a capture profile has no trigger schema.
    pub fn schema_unavailable() -> Self {
        Self {
            diagnostics: vec![diagnostic(
                "program.schema_id",
                TriggerValidationCode::SchemaUnavailable,
                "this capture profile does not advertise an advanced-trigger schema",
            )],
        }
    }
}

fn validate_operand(
    kind: &TriggerOperandKind,
    value: &TriggerOperandValue,
    path: &str,
    channels: &HashSet<&CaptureChannelId>,
    diagnostics: &mut Vec<TriggerValidationDiagnostic>,
) {
    match (kind, value) {
        (TriggerOperandKind::Boolean { .. }, TriggerOperandValue::Boolean(_)) => {}
        (
            TriggerOperandKind::Unsigned {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::Unsigned(value),
        )
        | (
            TriggerOperandKind::DurationNs {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::DurationNs(value),
        ) => validate_unsigned_value(*minimum, *maximum, *step, *value, path, diagnostics),
        (
            TriggerOperandKind::Signed {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::Signed(value),
        ) => validate_signed_value(*minimum, *maximum, *step, *value, path, diagnostics),
        (TriggerOperandKind::Choice { choices, .. }, TriggerOperandValue::Choice(value)) => {
            if !choices.iter().any(|choice| choice.id == *value) {
                diagnostics.push(diagnostic(
                    path,
                    TriggerValidationCode::UnknownChoice,
                    format!("trigger choice '{value}' is not registered"),
                ));
            }
        }
        (TriggerOperandKind::Channel { .. }, TriggerOperandValue::Channel(channel)) => {
            if !channels.contains(channel) {
                diagnostics.push(diagnostic(
                    path,
                    TriggerValidationCode::UnknownChannel,
                    format!("capture channel '{channel}' is not enabled"),
                ));
            }
        }
        (
            TriggerOperandKind::Bytes {
                minimum_length,
                maximum_length,
                ..
            },
            TriggerOperandValue::Bytes(value),
        ) => {
            if value.len() < *minimum_length || value.len() > *maximum_length {
                diagnostics.push(diagnostic(
                    path,
                    TriggerValidationCode::ByteLength,
                    format!(
                        "trigger byte value requires {}..={} bytes",
                        minimum_length, maximum_length
                    ),
                ));
            }
        }
        _ => diagnostics.push(diagnostic(
            path,
            TriggerValidationCode::OperandType,
            "trigger operand value has the wrong type",
        )),
    }
}

fn validate_unsigned_range(
    minimum: u64,
    maximum: u64,
    step: u64,
    default: u64,
) -> Result<(), TriggerSchemaError> {
    if minimum > maximum
        || step == 0
        || default < minimum
        || default > maximum
        || !(default - minimum).is_multiple_of(step)
    {
        return Err(TriggerSchemaError::InvalidUnsignedOperandRange);
    }
    Ok(())
}

fn validate_signed_range(
    minimum: i64,
    maximum: i64,
    step: u64,
    default: i64,
) -> Result<(), TriggerSchemaError> {
    if minimum > maximum || step == 0 || default < minimum || default > maximum {
        return Err(TriggerSchemaError::InvalidSignedOperandRange);
    }
    let offset = i128::from(default) - i128::from(minimum);
    if !u128::try_from(offset)
        .unwrap_or(u128::MAX)
        .is_multiple_of(u128::from(step))
    {
        return Err(TriggerSchemaError::InvalidSignedOperandStep);
    }
    Ok(())
}

fn validate_unsigned_value(
    minimum: u64,
    maximum: u64,
    step: u64,
    value: u64,
    path: &str,
    diagnostics: &mut Vec<TriggerValidationDiagnostic>,
) {
    if value < minimum || value > maximum {
        diagnostics.push(diagnostic(
            path,
            TriggerValidationCode::OperandRange,
            format!("trigger operand must be {minimum}..={maximum}"),
        ));
    } else if !(value - minimum).is_multiple_of(step) {
        diagnostics.push(diagnostic(
            path,
            TriggerValidationCode::OperandStep,
            format!("trigger operand must use step {step} from {minimum}"),
        ));
    }
}

fn validate_signed_value(
    minimum: i64,
    maximum: i64,
    step: u64,
    value: i64,
    path: &str,
    diagnostics: &mut Vec<TriggerValidationDiagnostic>,
) {
    if value < minimum || value > maximum {
        diagnostics.push(diagnostic(
            path,
            TriggerValidationCode::OperandRange,
            format!("trigger operand must be {minimum}..={maximum}"),
        ));
    } else {
        let offset = u128::try_from(i128::from(value) - i128::from(minimum)).unwrap_or(u128::MAX);
        if !offset.is_multiple_of(u128::from(step)) {
            diagnostics.push(diagnostic(
                path,
                TriggerValidationCode::OperandStep,
                format!("trigger operand must use step {step} from {minimum}"),
            ));
        }
    }
}

fn diagnostic(
    path: impl Into<String>,
    code: TriggerValidationCode,
    message: impl Into<String>,
) -> TriggerValidationDiagnostic {
    TriggerValidationDiagnostic {
        path: path.into(),
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> TriggerIdentifier {
        TriggerIdentifier::new(value).unwrap()
    }

    #[test]
    fn schema_construction_reports_classified_invariant_failures() {
        assert_eq!(
            TriggerIdentifier::new("invalid identifier").unwrap_err(),
            TriggerSchemaError::InvalidIdentifierCharacters {
                value: "invalid identifier".into(),
            }
        );
        assert_eq!(
            TriggerCountCapabilities::new(Vec::new(), 1, 1, 1).unwrap_err(),
            TriggerSchemaError::EmptyCountModes
        );
    }

    #[test]
    fn simple_program_representability_remains_a_schema_error() {
        let schema =
            TriggerEditorSchema::new(id("or-only"), 1, 1, 1, vec![TriggerLogicOperator::Or])
                .unwrap();

        assert_eq!(
            schema
                .simple_program([(
                    CaptureChannelId::new("pod-a:0"),
                    SimpleTriggerCondition::High,
                )])
                .unwrap_err(),
            TriggerSchemaError::UnsupportedSimpleProgram
        );
    }

    fn schema() -> TriggerEditorSchema {
        let serial = RegisteredTriggerPredicateSchema::new(
            id("test.serial"),
            "Serial word",
            vec![
                TriggerOperandSchema::new(
                    id("data"),
                    "Data channel",
                    TriggerOperandKind::Channel { default: None },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("edge"),
                    "Clock edge",
                    TriggerOperandKind::Choice {
                        choices: vec![
                            TriggerChoice::new(id("rising"), "Rising").unwrap(),
                            TriggerChoice::new(id("falling"), "Falling").unwrap(),
                        ],
                        default: id("rising"),
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("value"),
                    "Value",
                    TriggerOperandKind::Unsigned {
                        minimum: 0,
                        maximum: 255,
                        step: 2,
                        default: 0,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("armed"),
                    "Armed",
                    TriggerOperandKind::Boolean { default: true },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("offset"),
                    "Offset",
                    TriggerOperandKind::Signed {
                        minimum: -10,
                        maximum: 10,
                        step: 2,
                        default: 0,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("duration"),
                    "Duration",
                    TriggerOperandKind::DurationNs {
                        minimum: 100,
                        maximum: 1_000,
                        step: 100,
                        default: 100,
                    },
                )
                .unwrap(),
                TriggerOperandSchema::new(
                    id("pattern"),
                    "Pattern",
                    TriggerOperandKind::Bytes {
                        minimum_length: 1,
                        maximum_length: 4,
                        default: vec![0],
                    },
                )
                .unwrap(),
            ],
        )
        .unwrap();
        TriggerEditorSchema::new(
            id("test.engine"),
            3,
            4,
            8,
            vec![TriggerLogicOperator::And, TriggerLogicOperator::Or],
        )
        .unwrap()
        .with_digital_conditions(vec![
            SimpleTriggerCondition::High,
            SimpleTriggerCondition::Rising,
            SimpleTriggerCondition::Falling,
        ])
        .unwrap()
        .with_stage_inversion(true)
        .with_count(
            TriggerCountCapabilities::new(
                vec![TriggerCountMode::Occurrences, TriggerCountMode::Consecutive],
                1,
                16,
                1,
            )
            .unwrap(),
        )
        .with_registered_predicates(vec![serial])
        .unwrap()
    }

    fn channels() -> Vec<CaptureChannelId> {
        vec![
            CaptureChannelId::new("pod-a:3"),
            CaptureChannelId::new("bank-z:41"),
        ]
    }

    fn registered_predicate() -> TriggerPredicate {
        TriggerPredicate::Registered {
            predicate: id("test.serial"),
            operands: BTreeMap::from([
                (
                    id("data"),
                    TriggerOperandValue::Channel(CaptureChannelId::new("bank-z:41")),
                ),
                (id("edge"), TriggerOperandValue::Choice(id("falling"))),
                (id("value"), TriggerOperandValue::Unsigned(0x5a)),
                (id("armed"), TriggerOperandValue::Boolean(true)),
                (id("offset"), TriggerOperandValue::Signed(-2)),
                (id("duration"), TriggerOperandValue::DurationNs(300)),
                (id("pattern"), TriggerOperandValue::Bytes(vec![0x5a, 0xa5])),
            ]),
        }
    }

    fn valid_program() -> TriggerProgram {
        TriggerProgram::new(
            id("test.engine"),
            3,
            vec![
                TriggerStage {
                    predicates: vec![TriggerPredicate::Digital {
                        channel: CaptureChannelId::new("pod-a:3"),
                        condition: SimpleTriggerCondition::Rising,
                    }],
                    logic: TriggerLogicOperator::And,
                    inverted: false,
                    count: Some(TriggerCount {
                        mode: TriggerCountMode::Occurrences,
                        value: 4,
                    }),
                },
                TriggerStage {
                    predicates: vec![registered_predicate()],
                    logic: TriggerLogicOperator::Or,
                    inverted: true,
                    count: None,
                },
            ],
        )
    }

    #[test]
    fn accepts_supported_staged_counted_and_registered_program() {
        let program = valid_program();
        let validated = schema().validate_program(&program, &channels()).unwrap();
        assert_eq!(validated.program(), &program);
    }

    #[test]
    fn neutral_program_round_trips_through_serde() {
        let program = valid_program();
        let bytes = serde_json::to_vec(&program).unwrap();
        let decoded: TriggerProgram = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, program);
    }

    #[test]
    fn registered_identifiers_reject_invalid_serialized_values() {
        assert!(serde_json::from_str::<TriggerIdentifier>(r#""not a stable ID""#).is_err());
    }

    #[test]
    fn simple_program_uses_one_and_stage_and_omits_ignore() {
        let program = schema()
            .simple_program([
                (
                    CaptureChannelId::new("pod-a:3"),
                    SimpleTriggerCondition::Ignore,
                ),
                (
                    CaptureChannelId::new("bank-z:41"),
                    SimpleTriggerCondition::Falling,
                ),
            ])
            .unwrap()
            .unwrap();
        assert_eq!(program.stages.len(), 1);
        assert_eq!(program.stages[0].logic, TriggerLogicOperator::And);
        assert_eq!(program.stages[0].predicates.len(), 1);
        schema().validate_program(&program, &channels()).unwrap();
    }

    #[test]
    fn lane_edits_share_the_common_program_and_refuse_advanced_replacement() {
        let schema = schema();
        let channels = channels();
        let first = schema
            .with_simple_condition(
                None,
                &channels,
                &channels[0],
                SimpleTriggerCondition::Rising,
            )
            .unwrap();
        let second = schema
            .with_simple_condition(
                first.as_ref(),
                &channels,
                &channels[1],
                SimpleTriggerCondition::High,
            )
            .unwrap();
        let TriggerProgramForm::CommonDigital(conditions) =
            schema.program_form(second.as_ref(), &channels).unwrap()
        else {
            panic!("lane edits should produce the common digital form");
        };
        assert_eq!(conditions[&channels[0]], SimpleTriggerCondition::Rising);
        assert_eq!(conditions[&channels[1]], SimpleTriggerCondition::High);

        assert_eq!(
            schema
                .with_simple_condition(
                    Some(&valid_program()),
                    &channels,
                    &channels[0],
                    SimpleTriggerCondition::Falling,
                )
                .unwrap_err(),
            TriggerProgramEditError::AdvancedProgram
        );

        let cleared = schema
            .with_simple_condition(
                second.as_ref(),
                &channels,
                &channels[0],
                SimpleTriggerCondition::Ignore,
            )
            .unwrap();
        let cleared = schema
            .with_simple_condition(
                cleared.as_ref(),
                &channels,
                &channels[1],
                SimpleTriggerCondition::Ignore,
            )
            .unwrap();
        assert!(cleared.is_none());
    }

    #[test]
    fn rejects_duplicate_digital_channels_within_one_stage() {
        let schema = schema();
        let mut program = valid_program();
        let duplicate = program.stages[0].predicates[0].clone();
        program.stages[0].predicates.push(duplicate);

        let errors = schema.validate_program(&program, &channels()).unwrap_err();

        assert!(
            errors
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.code == TriggerValidationCode::DuplicateChannel)
        );
    }

    #[test]
    fn reports_schema_structure_channel_and_registered_operand_errors() {
        let mut program = valid_program();
        program.schema_revision = 2;
        program.stages[0].logic = TriggerLogicOperator::Nand;
        program.stages[0].inverted = true;
        program.stages[0].count = Some(TriggerCount {
            mode: TriggerCountMode::Occurrences,
            value: 99,
        });
        program.stages[0].predicates[0] = TriggerPredicate::Digital {
            channel: CaptureChannelId::new("missing:7"),
            condition: SimpleTriggerCondition::Either,
        };
        let TriggerPredicate::Registered { operands, .. } = &mut program.stages[1].predicates[0]
        else {
            unreachable!();
        };
        operands.remove(&id("edge"));
        operands.insert(id("value"), TriggerOperandValue::Unsigned(999));
        operands.insert(id("pattern"), TriggerOperandValue::Bytes(Vec::new()));
        operands.insert(id("extra"), TriggerOperandValue::Boolean(true));

        let errors = schema()
            .validate_program(&program, &channels())
            .unwrap_err();
        let codes: HashSet<_> = errors
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        for expected in [
            TriggerValidationCode::SchemaRevision,
            TriggerValidationCode::UnsupportedLogic,
            TriggerValidationCode::CountRange,
            TriggerValidationCode::UnknownChannel,
            TriggerValidationCode::UnsupportedDigitalCondition,
            TriggerValidationCode::MissingOperand,
            TriggerValidationCode::UnexpectedOperand,
            TriggerValidationCode::OperandRange,
            TriggerValidationCode::ByteLength,
        ] {
            assert!(
                codes.contains(&expected),
                "missing {expected:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn reports_format_predicate_type_step_and_choice_errors() {
        let mut program = valid_program();
        program.format_version = TRIGGER_PROGRAM_FORMAT_VERSION + 1;
        program.stages[0].predicates = vec![
            TriggerPredicate::Digital {
                channel: CaptureChannelId::new("pod-a:3"),
                condition: SimpleTriggerCondition::Rising,
            };
            9
        ];
        let TriggerPredicate::Registered { operands, .. } = &mut program.stages[1].predicates[0]
        else {
            unreachable!();
        };
        operands.insert(id("edge"), TriggerOperandValue::Choice(id("either")));
        operands.insert(id("value"), TriggerOperandValue::Unsigned(91));
        operands.insert(id("armed"), TriggerOperandValue::Unsigned(1));

        let errors = schema()
            .validate_program(&program, &channels())
            .unwrap_err();
        let codes: HashSet<_> = errors
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        for expected in [
            TriggerValidationCode::FormatVersion,
            TriggerValidationCode::PredicateLimit,
            TriggerValidationCode::UnknownChoice,
            TriggerValidationCode::OperandStep,
            TriggerValidationCode::OperandType,
        ] {
            assert!(
                codes.contains(&expected),
                "missing {expected:?}: {errors:?}"
            );
        }
    }

    #[test]
    fn materially_different_schema_rejects_unsupported_features() {
        let minimal = TriggerEditorSchema::new(
            id("minimal.engine"),
            1,
            1,
            1,
            vec![TriggerLogicOperator::And],
        )
        .unwrap()
        .with_digital_conditions(vec![SimpleTriggerCondition::High])
        .unwrap();
        let supported = minimal
            .simple_program([(
                CaptureChannelId::new("pod-a:3"),
                SimpleTriggerCondition::High,
            )])
            .unwrap()
            .unwrap();
        minimal.validate_program(&supported, &channels()).unwrap();

        let errors = minimal
            .validate_program(&valid_program(), &channels())
            .unwrap_err();
        let codes: HashSet<_> = errors
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&TriggerValidationCode::SchemaIdentity));
        assert!(codes.contains(&TriggerValidationCode::StageLimit));
        assert!(codes.contains(&TriggerValidationCode::UnsupportedLogic));
        assert!(codes.contains(&TriggerValidationCode::UnsupportedInversion));
        assert!(codes.contains(&TriggerValidationCode::UnsupportedCount));
        assert!(codes.contains(&TriggerValidationCode::UnknownPredicate));
    }

    #[test]
    fn reports_unsupported_count_mode_separately_from_count_range() {
        let schema =
            TriggerEditorSchema::new(id("count.engine"), 1, 1, 1, vec![TriggerLogicOperator::And])
                .unwrap()
                .with_digital_conditions(vec![SimpleTriggerCondition::High])
                .unwrap()
                .with_count(
                    TriggerCountCapabilities::new(vec![TriggerCountMode::Occurrences], 1, 9, 2)
                        .unwrap(),
                );
        let mut program = schema
            .simple_program([(
                CaptureChannelId::new("pod-a:3"),
                SimpleTriggerCondition::High,
            )])
            .unwrap()
            .unwrap();
        program.stages[0].count = Some(TriggerCount {
            mode: TriggerCountMode::Consecutive,
            value: 2,
        });

        let errors = schema.validate_program(&program, &channels()).unwrap_err();
        let codes: HashSet<_> = errors
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains(&TriggerValidationCode::UnsupportedCountMode));
        assert!(codes.contains(&TriggerValidationCode::CountRange));
    }
}
