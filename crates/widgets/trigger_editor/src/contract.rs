//! Host-facing trigger-editor declarations and results.
//!
//! These provider-neutral records describe available channels, requested
//! program edits, and render outcomes. They expose no reducer implementation,
//! egui state, device semantics, or application workflow.

use logic_analyzer_trigger::{
    SimpleTriggerCondition, TriggerCount, TriggerIdentifier, TriggerLogicOperator,
    TriggerOperandValue, TriggerProgram,
};
use signal_capture::CaptureChannelId;

/// One enabled capture channel offered by the trigger editor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerEditorChannel {
    pub id: CaptureChannelId,
    pub label: String,
}

/// User edit applied to a provider-neutral trigger program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerEditorAction {
    Clear,
    AddStage,
    RemoveStage {
        stage: usize,
    },
    SetStageLogic {
        stage: usize,
        logic: TriggerLogicOperator,
    },
    SetStageInverted {
        stage: usize,
        inverted: bool,
    },
    SetStageCount {
        stage: usize,
        count: Option<TriggerCount>,
    },
    AddDigitalPredicate {
        stage: usize,
        channel: CaptureChannelId,
        condition: SimpleTriggerCondition,
    },
    AddRegisteredPredicate {
        stage: usize,
        predicate: TriggerIdentifier,
    },
    RemovePredicate {
        stage: usize,
        predicate: usize,
    },
    SetDigitalChannel {
        stage: usize,
        predicate: usize,
        channel: CaptureChannelId,
    },
    SetDigitalCondition {
        stage: usize,
        predicate: usize,
        condition: SimpleTriggerCondition,
    },
    SetRegisteredOperand {
        stage: usize,
        predicate: usize,
        operand: TriggerIdentifier,
        value: TriggerOperandValue,
    },
}

#[derive(Default)]
pub struct TriggerEditorResponse {
    pub program: Option<Option<TriggerProgram>>,
    pub error: Option<String>,
}
