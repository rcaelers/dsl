//! Small panel title-bar controls.
//!
//! This module owns reusable egui widgets used by the layout title bar. It
//! consumes explicit panel metadata and does not select content, mutate the
//! layout tree, or interpret host identifiers.

use egui::{Color32, Rect, Sense, Stroke, Ui};

use super::contract::PanelSpec;

#[derive(Debug, Clone, Copy)]
pub(crate) enum PanelControlIcon {
    Maximize,
    RestoreLayout,
}

pub(crate) struct PanelContentButton<'a> {
    spec: PanelSpec<'a>,
    selected: bool,
}

pub(crate) fn panel_content_button(spec: PanelSpec<'_>, selected: bool) -> PanelContentButton<'_> {
    PanelContentButton { spec, selected }
}

impl egui::Widget for PanelContentButton<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let response = ui.add_sized(
            [190.0, 24.0],
            egui::Button::selectable(
                self.selected,
                egui::RichText::new(self.spec.title).color(Color32::TRANSPARENT),
            ),
        );
        let color = ui.visuals().widgets.style(&response).fg_stroke.color;
        let icon_rect = Rect::from_center_size(
            egui::pos2(response.rect.left() + 14.0, response.rect.center().y),
            egui::vec2(16.0, 16.0),
        );
        self.spec.icon.paint(ui, icon_rect, color);
        ui.painter().text(
            egui::pos2(response.rect.left() + 28.0, response.rect.center().y),
            egui::Align2::LEFT_CENTER,
            self.spec.title,
            egui::TextStyle::Button.resolve(ui.style()),
            color,
        );
        response
    }
}

pub(crate) fn panel_control_button(
    ui: &mut Ui,
    icon: PanelControlIcon,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 3.0, Color32::from_rgb(72, 72, 72));
    }
    let stroke = Stroke::new(1.5, ui.visuals().widgets.style(&response).fg_stroke.color);
    let center = rect.center();
    match icon {
        PanelControlIcon::Maximize => {
            let inner = rect.shrink(5.5);
            let arm = 3.0;
            for (corner, horizontal, vertical) in [
                (inner.left_top(), arm, arm),
                (inner.right_top(), -arm, arm),
                (inner.left_bottom(), arm, -arm),
                (inner.right_bottom(), -arm, -arm),
            ] {
                ui.painter().line_segment(
                    [corner, egui::pos2(corner.x + horizontal, corner.y)],
                    stroke,
                );
                ui.painter()
                    .line_segment([corner, egui::pos2(corner.x, corner.y + vertical)], stroke);
            }
        }
        PanelControlIcon::RestoreLayout => {
            ui.painter()
                .hline(center.x - 4.0..=center.x + 4.0, center.y, stroke);
            for direction in [-1.0, 1.0] {
                let outside = center.y + direction * 5.0;
                let inside = center.y + direction;
                ui.painter().line_segment(
                    [egui::pos2(center.x, outside), egui::pos2(center.x, inside)],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(center.x - 2.0, center.y + direction * 3.0),
                        egui::pos2(center.x, inside),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(center.x + 2.0, center.y + direction * 3.0),
                        egui::pos2(center.x, inside),
                    ],
                    stroke,
                );
            }
        }
    }
    response.on_hover_text(tooltip)
}
