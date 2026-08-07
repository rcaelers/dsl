//! Trigger value widgets, labels, and textual value conversion.
//!
//! This module owns presentation of provider-neutral operand kinds and their
//! temporary egui editing state. It emits generic values and contains no
//! program mutation, schema policy, device semantics, or application workflow.

use signal_capture_session::{
    CaptureChannelId, SimpleTriggerCondition, TriggerCountMode, TriggerLogicOperator,
    TriggerOperandKind, TriggerOperandValue,
};

use super::contract::TriggerEditorChannel;

pub(crate) fn show_operand(
    ui: &mut egui::Ui,
    enabled: bool,
    channels: &[TriggerEditorChannel],
    id: (usize, usize, &str),
    kind: &TriggerOperandKind,
    value: &TriggerOperandValue,
) -> Option<TriggerOperandValue> {
    match (kind, value) {
        (TriggerOperandKind::Boolean { .. }, TriggerOperandValue::Boolean(value)) => {
            let mut updated = *value;
            ui.add_enabled(enabled, egui::Checkbox::without_text(&mut updated))
                .changed()
                .then_some(TriggerOperandValue::Boolean(updated))
        }
        (
            TriggerOperandKind::Unsigned {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::Unsigned(value),
        ) => unsigned_operand(ui, enabled, *minimum, *maximum, *step, *value)
            .map(TriggerOperandValue::Unsigned),
        (
            TriggerOperandKind::DurationNs {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::DurationNs(value),
        ) => unsigned_operand(ui, enabled, *minimum, *maximum, *step, *value)
            .map(TriggerOperandValue::DurationNs),
        (
            TriggerOperandKind::Signed {
                minimum,
                maximum,
                step,
                ..
            },
            TriggerOperandValue::Signed(value),
        ) => {
            let mut updated = *value;
            ui.add_enabled(
                enabled,
                egui::DragValue::new(&mut updated)
                    .range(*minimum..=*maximum)
                    .speed(*step as f64),
            )
            .changed()
            .then_some(TriggerOperandValue::Signed(updated))
        }
        (TriggerOperandKind::Choice { choices, .. }, TriggerOperandValue::Choice(value)) => {
            let mut updated = None;
            let selected = choices
                .iter()
                .find(|choice| choice.id() == value)
                .map_or(value.as_str(), |choice| choice.label());
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt(("trigger-choice", id))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            if ui
                                .selectable_label(choice.id() == value, choice.label())
                                .clicked()
                            {
                                updated = Some(TriggerOperandValue::Choice(choice.id().clone()));
                            }
                        }
                    });
            });
            updated
        }
        (TriggerOperandKind::Channel { .. }, TriggerOperandValue::Channel(value)) => {
            let mut updated = None;
            ui.add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt(("trigger-channel", id))
                    .selected_text(channel_label(channels, value))
                    .show_ui(ui, |ui| {
                        for channel in channels {
                            if ui
                                .selectable_label(channel.id == *value, &channel.label)
                                .clicked()
                            {
                                updated = Some(TriggerOperandValue::Channel(channel.id.clone()));
                            }
                        }
                    });
            });
            updated
        }
        (
            TriggerOperandKind::Bytes {
                minimum_length,
                maximum_length,
                ..
            },
            TriggerOperandValue::Bytes(value),
        ) => {
            let memory_id = ui.make_persistent_id(("trigger-bytes", id));
            let canonical = format_bytes(value);
            let mut text = ui
                .data(|data| data.get_temp::<String>(memory_id))
                .unwrap_or(canonical.clone());
            let response = ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut text).desired_width(120.0),
            );
            if response.changed() {
                ui.data_mut(|data| data.insert_temp(memory_id, text.clone()));
                parse_bytes(&text)
                    .filter(|bytes| {
                        bytes.len() >= *minimum_length && bytes.len() <= *maximum_length
                    })
                    .map(TriggerOperandValue::Bytes)
            } else {
                if !response.has_focus() && text != canonical {
                    ui.data_mut(|data| data.remove::<String>(memory_id));
                }
                None
            }
        }
        _ => {
            ui.colored_label(ui.visuals().error_fg_color, "wrong operand type");
            None
        }
    }
}
fn unsigned_operand(
    ui: &mut egui::Ui,
    enabled: bool,
    minimum: u64,
    maximum: u64,
    step: u64,
    value: u64,
) -> Option<u64> {
    let mut updated = value;
    ui.add_enabled(
        enabled,
        egui::DragValue::new(&mut updated)
            .range(minimum..=maximum)
            .speed(step as f64),
    )
    .changed()
    .then_some(updated)
}

pub(crate) fn channel_label<'a>(
    channels: &'a [TriggerEditorChannel],
    id: &CaptureChannelId,
) -> &'a str {
    channels
        .iter()
        .find(|channel| channel.id == *id)
        .map_or("Unknown channel", |channel| channel.label.as_str())
}

pub(crate) const fn logic_label(logic: TriggerLogicOperator) -> &'static str {
    match logic {
        TriggerLogicOperator::And => "AND",
        TriggerLogicOperator::Or => "OR",
        TriggerLogicOperator::Xor => "XOR",
        TriggerLogicOperator::Nand => "NAND",
        TriggerLogicOperator::Nor => "NOR",
    }
}

pub(crate) const fn count_mode_label(mode: TriggerCountMode) -> &'static str {
    match mode {
        TriggerCountMode::Occurrences => "Occurrences",
        TriggerCountMode::Consecutive => "Consecutive",
    }
}

pub(crate) const fn condition_label(condition: SimpleTriggerCondition) -> &'static str {
    match condition {
        SimpleTriggerCondition::Ignore => "Ignore",
        SimpleTriggerCondition::Low => "Low",
        SimpleTriggerCondition::High => "High",
        SimpleTriggerCondition::Rising => "Rising edge",
        SimpleTriggerCondition::Falling => "Falling edge",
        SimpleTriggerCondition::Either => "Either edge",
    }
}

pub(crate) fn format_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn parse_bytes(value: &str) -> Option<Vec<u8>> {
    value
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect()
}
