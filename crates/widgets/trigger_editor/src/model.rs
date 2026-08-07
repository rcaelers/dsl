//! Validated trigger-program edit reducer.
//!
//! This module owns default construction, indexed edit application, schema
//! validation, and reducer diagnostics. It consumes provider-neutral contracts
//! and does not render egui controls or define device semantics.

use std::collections::BTreeMap;

use signal_capture_session::{
    CaptureChannelId, RegisteredTriggerPredicateSchema, TriggerEditorSchema, TriggerIdentifier,
    TriggerOperandKind, TriggerOperandValue, TriggerPredicate, TriggerProgram, TriggerStage,
};

use super::contract::{TriggerEditorAction, TriggerEditorChannel};

pub struct TriggerEditorModel<'a> {
    schema: &'a TriggerEditorSchema,
    channels: &'a [TriggerEditorChannel],
}

impl<'a> TriggerEditorModel<'a> {
    /// Creates a trigger-program editor model for one current schema revision.
    ///
    /// # Parameters
    /// - `schema`: Provider-advertised trigger grammar and limits.
    /// - `channels`: Currently enabled capture channels available to predicates.
    pub const fn new(
        schema: &'a TriggerEditorSchema,
        channels: &'a [TriggerEditorChannel],
    ) -> Self {
        Self { schema, channels }
    }

    /// Applies one edit, validates the result, and returns the replacement program.
    ///
    /// # Parameters
    /// - `current`: Existing program, or `None` when no trigger is configured.
    /// - `action`: Requested structural or value edit.
    ///
    /// Returns `Ok(None)` when the edit clears the last trigger stage.
    pub fn apply(
        &self,
        current: Option<&TriggerProgram>,
        action: TriggerEditorAction,
    ) -> Result<Option<TriggerProgram>, String> {
        if action == TriggerEditorAction::Clear {
            return Ok(None);
        }
        if let Some(current) = current {
            self.schema
                .validate_program(current, &self.channel_ids())
                .map_err(|error| error.to_string())?;
        }
        let mut program = current.cloned().unwrap_or_else(|| {
            TriggerProgram::new(self.schema.id().clone(), self.schema.revision(), Vec::new())
        });
        match action {
            TriggerEditorAction::Clear => unreachable!(),
            TriggerEditorAction::AddStage => {
                if program.stages.len() >= self.schema.maximum_stages() {
                    return Err(format!(
                        "this trigger schema supports at most {} stage(s)",
                        self.schema.maximum_stages()
                    ));
                }
                program.stages.push(self.default_stage()?);
            }
            TriggerEditorAction::RemoveStage { stage } => {
                checked_remove(&mut program.stages, stage, "trigger stage")?;
                if program.stages.is_empty() {
                    return Ok(None);
                }
            }
            TriggerEditorAction::SetStageLogic { stage, logic } => {
                self.stage_mut(&mut program, stage)?.logic = logic;
            }
            TriggerEditorAction::SetStageInverted { stage, inverted } => {
                self.stage_mut(&mut program, stage)?.inverted = inverted;
            }
            TriggerEditorAction::SetStageCount { stage, count } => {
                self.stage_mut(&mut program, stage)?.count = count;
            }
            TriggerEditorAction::AddDigitalPredicate {
                stage,
                channel,
                condition,
            } => {
                self.ensure_predicate_capacity(&program, stage)?;
                self.stage_mut(&mut program, stage)?
                    .predicates
                    .push(TriggerPredicate::Digital { channel, condition });
            }
            TriggerEditorAction::AddRegisteredPredicate { stage, predicate } => {
                self.ensure_predicate_capacity(&program, stage)?;
                let predicate_schema = self
                    .schema
                    .registered_predicates()
                    .iter()
                    .find(|candidate| candidate.id() == &predicate)
                    .ok_or_else(|| {
                        format!("registered trigger predicate '{predicate}' is unknown")
                    })?;
                let operands = self.default_operands(predicate_schema)?;
                self.stage_mut(&mut program, stage)?.predicates.push(
                    TriggerPredicate::Registered {
                        predicate,
                        operands,
                    },
                );
            }
            TriggerEditorAction::RemovePredicate { stage, predicate } => {
                let stage_ref = self.stage_mut(&mut program, stage)?;
                checked_remove(&mut stage_ref.predicates, predicate, "trigger predicate")?;
                if stage_ref.predicates.is_empty() {
                    program.stages.remove(stage);
                    if program.stages.is_empty() {
                        return Ok(None);
                    }
                }
            }
            TriggerEditorAction::SetDigitalChannel {
                stage,
                predicate,
                channel,
            } => {
                let TriggerPredicate::Digital {
                    channel: current, ..
                } = self.predicate_mut(&mut program, stage, predicate)?
                else {
                    return Err("the selected predicate is not a digital condition".into());
                };
                *current = channel;
            }
            TriggerEditorAction::SetDigitalCondition {
                stage,
                predicate,
                condition,
            } => {
                let TriggerPredicate::Digital {
                    condition: current, ..
                } = self.predicate_mut(&mut program, stage, predicate)?
                else {
                    return Err("the selected predicate is not a digital condition".into());
                };
                *current = condition;
            }
            TriggerEditorAction::SetRegisteredOperand {
                stage,
                predicate,
                operand,
                value,
            } => {
                let TriggerPredicate::Registered { operands, .. } =
                    self.predicate_mut(&mut program, stage, predicate)?
                else {
                    return Err("the selected predicate is not registered".into());
                };
                let Some(current) = operands.get_mut(&operand) else {
                    return Err(format!("registered trigger operand '{operand}' is unknown"));
                };
                *current = value;
            }
        }
        self.schema
            .validate_program(&program, &self.channel_ids())
            .map_err(|error| error.to_string())?;
        Ok(Some(program))
    }

    fn channel_ids(&self) -> Vec<CaptureChannelId> {
        self.channels
            .iter()
            .map(|channel| channel.id.clone())
            .collect()
    }

    fn default_stage(&self) -> Result<TriggerStage, String> {
        let predicate = if let (Some(channel), Some(condition)) = (
            self.channels.first(),
            self.schema.digital_conditions().first(),
        ) {
            TriggerPredicate::Digital {
                channel: channel.id.clone(),
                condition: *condition,
            }
        } else if let Some(predicate) = self.schema.registered_predicates().first() {
            TriggerPredicate::Registered {
                predicate: predicate.id().clone(),
                operands: self.default_operands(predicate)?,
            }
        } else {
            return Err("this trigger schema has no predicate available for a new stage".into());
        };
        Ok(TriggerStage {
            predicates: vec![predicate],
            logic: *self
                .schema
                .logic_operators()
                .first()
                .ok_or_else(|| "this trigger schema has no stage logic".to_owned())?,
            inverted: false,
            count: None,
        })
    }

    fn default_operands(
        &self,
        predicate: &RegisteredTriggerPredicateSchema,
    ) -> Result<BTreeMap<TriggerIdentifier, TriggerOperandValue>, String> {
        predicate
            .operands()
            .iter()
            .map(|operand| {
                self.default_operand(operand.kind())
                    .map(|value| (operand.id().clone(), value))
            })
            .collect()
    }

    fn default_operand(&self, kind: &TriggerOperandKind) -> Result<TriggerOperandValue, String> {
        Ok(match kind {
            TriggerOperandKind::Boolean { default } => TriggerOperandValue::Boolean(*default),
            TriggerOperandKind::Unsigned { default, .. } => TriggerOperandValue::Unsigned(*default),
            TriggerOperandKind::Signed { default, .. } => TriggerOperandValue::Signed(*default),
            TriggerOperandKind::DurationNs { default, .. } => {
                TriggerOperandValue::DurationNs(*default)
            }
            TriggerOperandKind::Choice { default, .. } => {
                TriggerOperandValue::Choice(default.clone())
            }
            TriggerOperandKind::Channel { default } => {
                let channel = default
                    .as_ref()
                    .filter(|default| self.channels.iter().any(|channel| channel.id == **default))
                    .cloned()
                    .or_else(|| self.channels.first().map(|channel| channel.id.clone()))
                    .ok_or_else(|| "a channel operand requires an enabled channel".to_owned())?;
                TriggerOperandValue::Channel(channel)
            }
            TriggerOperandKind::Bytes { default, .. } => {
                TriggerOperandValue::Bytes(default.clone())
            }
        })
    }

    fn stage_mut<'program>(
        &self,
        program: &'program mut TriggerProgram,
        stage: usize,
    ) -> Result<&'program mut TriggerStage, String> {
        program
            .stages
            .get_mut(stage)
            .ok_or_else(|| format!("trigger stage {stage} does not exist"))
    }

    fn predicate_mut<'program>(
        &self,
        program: &'program mut TriggerProgram,
        stage: usize,
        predicate: usize,
    ) -> Result<&'program mut TriggerPredicate, String> {
        self.stage_mut(program, stage)?
            .predicates
            .get_mut(predicate)
            .ok_or_else(|| format!("trigger predicate {stage}:{predicate} does not exist"))
    }

    fn ensure_predicate_capacity(
        &self,
        program: &TriggerProgram,
        stage: usize,
    ) -> Result<(), String> {
        let stage = program
            .stages
            .get(stage)
            .ok_or_else(|| format!("trigger stage {stage} does not exist"))?;
        if stage.predicates.len() >= self.schema.maximum_predicates_per_stage() {
            return Err(format!(
                "this trigger schema supports at most {} predicate(s) per stage",
                self.schema.maximum_predicates_per_stage()
            ));
        }
        Ok(())
    }
}

fn checked_remove<T>(values: &mut Vec<T>, index: usize, label: &str) -> Result<T, String> {
    if index >= values.len() {
        return Err(format!("{label} {index} does not exist"));
    }
    Ok(values.remove(index))
}
