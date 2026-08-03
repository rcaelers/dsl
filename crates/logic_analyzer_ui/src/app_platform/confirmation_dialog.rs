pub(crate) const ACCENT_COLOR: egui::Color32 = egui::Color32::from_rgb(240, 180, 70);
pub(crate) const DESTRUCTIVE_BUTTON_COLOR: egui::Color32 = egui::Color32::from_rgb(135, 55, 50);
pub(crate) const DESTRUCTIVE_TEXT_COLOR: egui::Color32 = egui::Color32::from_rgb(245, 175, 165);

pub(crate) struct DestructiveConfirmation<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub message: &'a str,
    pub detail: &'a str,
    pub confirm_label: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmationChoice {
    Confirm,
    Cancel,
}

pub(crate) fn show_prominent_modal<R>(
    ctx: &egui::Context,
    id: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::ModalResponse<R> {
    let style = ctx.style_of(ctx.theme());
    egui::Modal::new(egui::Id::new(id))
        .backdrop_color(egui::Color32::from_black_alpha(190))
        .frame(
            egui::Frame::popup(&style)
                .fill(egui::Color32::from_rgb(47, 39, 25))
                .stroke(egui::Stroke::new(2.0, ACCENT_COLOR))
                .inner_margin(egui::Margin::symmetric(28, 24)),
        )
        .show(ctx, |ui| {
            ui.set_min_width(430.0);
            add_contents(ui)
        })
}

pub(crate) fn show_destructive_confirmation(
    ctx: &egui::Context,
    confirmation: DestructiveConfirmation<'_>,
) -> Option<ConfirmationChoice> {
    let mut choice = None;
    let modal = show_prominent_modal(ctx, confirmation.id, |ui| {
        ui.label(
            egui::RichText::new(confirmation.title)
                .size(26.0)
                .strong()
                .color(ACCENT_COLOR),
        );
        ui.add_space(8.0);
        ui.label(egui::RichText::new(confirmation.message).size(16.0));
        ui.add_space(6.0);
        ui.label(egui::RichText::new(confirmation.detail).color(DESTRUCTIVE_TEXT_COLOR));
        ui.add_space(20.0);

        ui.horizontal(|ui| {
            if ui
                .add_sized([108.0, 32.0], egui::Button::new("Cancel"))
                .clicked()
            {
                choice = Some(ConfirmationChoice::Cancel);
            }
            let remaining_width = ui.available_width();
            ui.allocate_ui_with_layout(
                egui::Vec2::new(remaining_width, 32.0),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui
                        .add_sized(
                            [132.0, 32.0],
                            egui::Button::new(confirmation.confirm_label)
                                .fill(DESTRUCTIVE_BUTTON_COLOR),
                        )
                        .clicked()
                    {
                        choice = Some(ConfirmationChoice::Confirm);
                    }
                },
            );
        });
    });

    if choice.is_none() && modal.should_close() {
        choice = Some(ConfirmationChoice::Cancel);
    }
    choice
}

#[cfg(test)]
mod confirmation_dialog_tests {
    use super::*;

    #[test]
    fn escape_cancels_a_destructive_confirmation() {
        let ctx = egui::Context::default();
        let confirmation = || DestructiveConfirmation {
            id: "test-confirmation",
            title: "Clear data?",
            message: "Data will be removed.",
            detail: "This cannot be undone.",
            confirm_label: "Clear",
        };
        ctx.begin_pass(Default::default());
        assert_eq!(show_destructive_confirmation(&ctx, confirmation()), None);
        let _ = ctx.end_pass();

        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: Some(egui::Key::Escape),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        });

        let choice = show_destructive_confirmation(&ctx, confirmation());
        let _ = ctx.end_pass();

        assert_eq!(choice, Some(ConfirmationChoice::Cancel));
    }
}
