//! Platform-neutral trigger configuration for logic-analyzer sources.

use logic_analyzer_trigger::TriggerCountMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerCondition {
    /// Do not constrain this channel in the stage.
    Ignore,
    /// Require a low level.
    Low,
    /// Require a high level.
    High,
    /// Require a low-to-high transition.
    Rising,
    /// Require a high-to-low transition.
    Falling,
    /// Require either edge transition.
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerLogic {
    /// Require all active conditions across trigger planes.
    And,
    /// Require any active condition across trigger planes.
    Or,
}

/// One stage of a portable logic trigger. Two planes accommodate analyzers
/// with parallel trigger match units; one-plane drivers reject plane1.
#[derive(Debug, Clone)]
pub struct LogicTriggerStage {
    /// Conditions assigned to the driver's first trigger-match plane.
    pub plane0: [TriggerCondition; 16],
    /// Conditions assigned to the optional second trigger-match plane.
    pub plane1: [TriggerCondition; 16],
    /// Boolean operator joining the planes.
    pub logic: TriggerLogic,
    /// Whether to invert the stage result.
    pub inverted: bool,
    /// Interpretation of [`Self::count`].
    pub count_mode: TriggerCountMode,
    /// Occurrence or elapsed-time threshold selected by [`Self::count_mode`].
    pub count: u32,
}

impl Default for LogicTriggerStage {
    fn default() -> Self {
        Self {
            plane0: [TriggerCondition::Ignore; 16],
            plane1: [TriggerCondition::Ignore; 16],
            logic: TriggerLogic::And,
            inverted: false,
            count_mode: TriggerCountMode::Occurrences,
            count: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LogicTrigger {
    /// Ordered hardware trigger stages.
    pub stages: Vec<LogicTriggerStage>,
    /// Whether stages form a serial sequence instead of independent matches.
    pub serial: bool,
}
