//! Egui trigger-editor composition and presentation.
//!
//! This module owns rendering and the translation of widget changes into
//! generic editor actions. The crate root exposes its supported public types.
//! It consumes only provider-neutral trigger contracts and excludes device
//! semantics, acquisition behavior, and application workflow.

use logic_analyzer_trigger::{
    TriggerCount, TriggerEditorSchema, TriggerPredicate, TriggerProgram, TriggerStage,
};

use super::contract::{TriggerEditorAction, TriggerEditorChannel, TriggerEditorResponse};
use super::model::TriggerEditorModel;
use super::presentation::{
    channel_label, condition_label, count_mode_label, logic_label, show_operand,
};

pub struct TriggerEditor<'a> {
    schema: &'a TriggerEditorSchema,
    channels: &'a [TriggerEditorChannel],
    program: Option<&'a TriggerProgram>,
    enabled: bool,
}

impl<'a> TriggerEditor<'a> {
    /// Creates an egui trigger editor bound to a schema, channels, and optional program.
    ///
    /// # Parameters
    /// - `schema`: Provider-advertised trigger grammar and limits.
    /// - `channels`: Currently enabled capture channels available to predicates.
    /// - `program`: Existing program to display, if one is configured.
    pub const fn new(
        schema: &'a TriggerEditorSchema,
        channels: &'a [TriggerEditorChannel],
        program: Option<&'a TriggerProgram>,
    ) -> Self {
        Self {
            schema,
            channels,
            program,
            enabled: true,
        }
    }

    /// Enables or disables interactive editing while retaining read-only presentation.
    ///
    /// # Parameters
    /// - `enabled`: Whether the editor may emit program-changing actions.
    pub const fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draws the editor and returns at most one validated program replacement or error.
    ///
    /// # Parameters
    /// - `ui`: Egui UI allocated to the editor.
    pub fn show(self, ui: &mut egui::Ui) -> TriggerEditorResponse {
        let mut action = None;
        let validation = self
            .program
            .map(|program| {
                self.schema.validate_program(
                    program,
                    &self
                        .channels
                        .iter()
                        .map(|channel| channel.id.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .transpose();
        if let Err(errors) = validation {
            for diagnostic in errors.diagnostics() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!("{}: {}", diagnostic.path, diagnostic.message),
                );
            }
            if ui
                .add_enabled(
                    self.enabled,
                    egui::Button::new("Clear incompatible trigger"),
                )
                .clicked()
            {
                action = Some(TriggerEditorAction::Clear);
            }
        } else if let Some(program) = self.program {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.enabled, egui::Button::new("Clear Trigger"))
                    .clicked()
                {
                    action = Some(TriggerEditorAction::Clear);
                }
                if ui
                    .add_enabled(
                        self.enabled && program.stages.len() < self.schema.maximum_stages(),
                        egui::Button::new("+ Stage"),
                    )
                    .clicked()
                {
                    action = Some(TriggerEditorAction::AddStage);
                }
            });
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (stage_index, stage) in program.stages.iter().enumerate() {
                    ui.group(|ui| {
                        self.show_stage(ui, stage_index, stage, &mut action);
                    });
                    ui.add_space(4.0);
                }
            });
        } else {
            ui.label(egui::RichText::new("Free run — no trigger program").weak());
            if ui
                .add_enabled(self.enabled, egui::Button::new("Add Trigger"))
                .clicked()
            {
                action = Some(TriggerEditorAction::AddStage);
            }
        }

        let Some(action) = action else {
            return TriggerEditorResponse::default();
        };
        match TriggerEditorModel::new(self.schema, self.channels).apply(self.program, action) {
            Ok(program) => TriggerEditorResponse {
                program: Some(program),
                error: None,
            },
            Err(error) => TriggerEditorResponse {
                program: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn show_stage(
        &self,
        ui: &mut egui::Ui,
        stage_index: usize,
        stage: &TriggerStage,
        action: &mut Option<TriggerEditorAction>,
    ) {
        ui.horizontal(|ui| {
            ui.strong(format!("Stage {}", stage_index + 1));
            ui.add_enabled_ui(self.enabled, |ui| {
                egui::ComboBox::from_id_salt(("trigger-stage-logic", stage_index))
                    .selected_text(logic_label(stage.logic))
                    .show_ui(ui, |ui| {
                        for logic in self.schema.logic_operators() {
                            if ui
                                .selectable_label(*logic == stage.logic, logic_label(*logic))
                                .clicked()
                            {
                                *action = Some(TriggerEditorAction::SetStageLogic {
                                    stage: stage_index,
                                    logic: *logic,
                                });
                            }
                        }
                    });
            });
            if self.schema.supports_stage_inversion() {
                let mut inverted = stage.inverted;
                if ui
                    .add_enabled(self.enabled, egui::Checkbox::new(&mut inverted, "Invert"))
                    .changed()
                {
                    *action = Some(TriggerEditorAction::SetStageInverted {
                        stage: stage_index,
                        inverted,
                    });
                }
            }
            if ui
                .add_enabled(self.enabled, egui::Button::new("Remove"))
                .clicked()
            {
                *action = Some(TriggerEditorAction::RemoveStage { stage: stage_index });
            }
        });
        self.show_count(ui, stage_index, stage, action);
        for (predicate_index, predicate) in stage.predicates.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                self.show_predicate(ui, stage_index, predicate_index, predicate, action);
                if ui
                    .add_enabled(self.enabled, egui::Button::new("×"))
                    .on_hover_text("Remove condition")
                    .clicked()
                {
                    *action = Some(TriggerEditorAction::RemovePredicate {
                        stage: stage_index,
                        predicate: predicate_index,
                    });
                }
            });
        }
        let unused_digital_channel = self.channels.iter().find(|channel| {
            !stage.predicates.iter().any(|predicate| {
                matches!(
                    predicate,
                    TriggerPredicate::Digital { channel: used, .. } if *used == channel.id
                )
            })
        });
        let can_add_digital =
            unused_digital_channel.is_some() && !self.schema.digital_conditions().is_empty();
        let has_condition_kind = can_add_digital || !self.schema.registered_predicates().is_empty();
        ui.add_enabled_ui(
            self.enabled
                && has_condition_kind
                && stage.predicates.len() < self.schema.maximum_predicates_per_stage(),
            |ui| {
                ui.menu_button("+ Condition", |ui| {
                    if let (Some(channel), Some(condition)) = (
                        unused_digital_channel,
                        self.schema.digital_conditions().first(),
                    ) && ui.button("Digital condition").clicked()
                    {
                        *action = Some(TriggerEditorAction::AddDigitalPredicate {
                            stage: stage_index,
                            channel: channel.id.clone(),
                            condition: *condition,
                        });
                        ui.close();
                    }
                    for predicate in self.schema.registered_predicates() {
                        if ui.button(predicate.label()).clicked() {
                            *action = Some(TriggerEditorAction::AddRegisteredPredicate {
                                stage: stage_index,
                                predicate: predicate.id().clone(),
                            });
                            ui.close();
                        }
                    }
                });
            },
        );
    }

    fn show_count(
        &self,
        ui: &mut egui::Ui,
        stage_index: usize,
        stage: &TriggerStage,
        action: &mut Option<TriggerEditorAction>,
    ) {
        let Some(capabilities) = self.schema.count_capabilities() else {
            return;
        };
        ui.horizontal(|ui| {
            let mut enabled = stage.count.is_some();
            if ui
                .add_enabled(self.enabled, egui::Checkbox::new(&mut enabled, "Count"))
                .changed()
            {
                let count = enabled.then(|| TriggerCount {
                    mode: capabilities.modes()[0],
                    value: capabilities.minimum(),
                });
                *action = Some(TriggerEditorAction::SetStageCount {
                    stage: stage_index,
                    count,
                });
            }
            let Some(count) = stage.count else {
                return;
            };
            ui.add_enabled_ui(self.enabled, |ui| {
                egui::ComboBox::from_id_salt(("trigger-count-mode", stage_index))
                    .selected_text(count_mode_label(count.mode))
                    .show_ui(ui, |ui| {
                        for mode in capabilities.modes() {
                            if ui
                                .selectable_label(*mode == count.mode, count_mode_label(*mode))
                                .clicked()
                            {
                                *action = Some(TriggerEditorAction::SetStageCount {
                                    stage: stage_index,
                                    count: Some(TriggerCount {
                                        mode: *mode,
                                        value: count.value,
                                    }),
                                });
                            }
                        }
                    });
            });
            let mut value = count.value;
            if ui
                .add_enabled(
                    self.enabled,
                    egui::DragValue::new(&mut value)
                        .range(capabilities.minimum()..=capabilities.maximum())
                        .speed(capabilities.step() as f64),
                )
                .changed()
            {
                *action = Some(TriggerEditorAction::SetStageCount {
                    stage: stage_index,
                    count: Some(TriggerCount {
                        mode: count.mode,
                        value,
                    }),
                });
            }
        });
    }

    fn show_predicate(
        &self,
        ui: &mut egui::Ui,
        stage_index: usize,
        predicate_index: usize,
        predicate: &TriggerPredicate,
        action: &mut Option<TriggerEditorAction>,
    ) {
        match predicate {
            TriggerPredicate::Digital { channel, condition } => {
                ui.add_enabled_ui(self.enabled, |ui| {
                    egui::ComboBox::from_id_salt((
                        "trigger-digital-channel",
                        stage_index,
                        predicate_index,
                    ))
                    .selected_text(channel_label(self.channels, channel))
                    .show_ui(ui, |ui| {
                        for candidate in self.channels {
                            if ui
                                .selectable_label(candidate.id == *channel, &candidate.label)
                                .clicked()
                            {
                                *action = Some(TriggerEditorAction::SetDigitalChannel {
                                    stage: stage_index,
                                    predicate: predicate_index,
                                    channel: candidate.id.clone(),
                                });
                            }
                        }
                    });
                });
                ui.add_enabled_ui(self.enabled, |ui| {
                    egui::ComboBox::from_id_salt((
                        "trigger-digital-condition",
                        stage_index,
                        predicate_index,
                    ))
                    .selected_text(condition_label(*condition))
                    .show_ui(ui, |ui| {
                        for candidate in self.schema.digital_conditions() {
                            if ui
                                .selectable_label(
                                    *candidate == *condition,
                                    condition_label(*candidate),
                                )
                                .clicked()
                            {
                                *action = Some(TriggerEditorAction::SetDigitalCondition {
                                    stage: stage_index,
                                    predicate: predicate_index,
                                    condition: *candidate,
                                });
                            }
                        }
                    });
                });
            }
            TriggerPredicate::Registered {
                predicate,
                operands,
            } => {
                let Some(predicate_schema) = self
                    .schema
                    .registered_predicates()
                    .iter()
                    .find(|candidate| candidate.id() == predicate)
                else {
                    ui.colored_label(ui.visuals().error_fg_color, predicate.as_str());
                    return;
                };
                ui.strong(predicate_schema.label());
                for operand in predicate_schema.operands() {
                    ui.label(operand.label());
                    if let Some(value) = operands.get(operand.id())
                        && let Some(updated) = show_operand(
                            ui,
                            self.enabled,
                            self.channels,
                            (stage_index, predicate_index, operand.id().as_str()),
                            operand.kind(),
                            value,
                        )
                    {
                        *action = Some(TriggerEditorAction::SetRegisteredOperand {
                            stage: stage_index,
                            predicate: predicate_index,
                            operand: operand.id().clone(),
                            value: updated,
                        });
                    }
                }
            }
        }
    }
}
