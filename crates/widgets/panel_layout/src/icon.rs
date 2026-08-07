//! Application-neutral panel icon selection and rendering.
//!
//! Hosts choose icons explicitly. This module owns only the reusable icon
//! vocabulary and its egui presentation; it never interprets panel identity.

use egui::{Color32, Rect, Stroke, StrokeKind, Ui};

/// Application-neutral vector icons for panel content selectors.
///
/// Hosts explicitly select an icon when declaring a [`crate::PanelSpec`]; the panel
/// manager never derives presentation from content identifiers or titles.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PanelIcon {
    #[default]
    Panel,
    Waveform,
    Network,
    Image,
    List,
    Console,
    Eye,
    Chip,
    Target,
    Table,
    Reset,
}

impl PanelIcon {
    pub(crate) fn paint(self, ui: &Ui, rect: Rect, color: Color32) {
        let painter = ui.painter();
        let rect = Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
        let stroke = Stroke::new(1.5, color);
        match self {
            Self::Panel => {
                let panel = rect.shrink(2.0);
                painter.rect_stroke(panel, 2.0, stroke, StrokeKind::Inside);
                painter.line_segment(
                    [
                        egui::pos2(panel.left(), panel.top() + 4.0),
                        egui::pos2(panel.right(), panel.top() + 4.0),
                    ],
                    stroke,
                );
            }
            Self::Waveform => {
                let left = rect.left() + 1.0;
                let right = rect.right() - 1.0;
                let high = rect.top() + 4.0;
                let low = rect.bottom() - 4.0;
                let quarter = (right - left) * 0.25;
                painter.add(egui::Shape::line(
                    vec![
                        egui::pos2(left, low),
                        egui::pos2(left + quarter, low),
                        egui::pos2(left + quarter, high),
                        egui::pos2(left + quarter * 3.0, high),
                        egui::pos2(left + quarter * 3.0, low),
                        egui::pos2(right, low),
                    ],
                    stroke,
                ));
            }
            Self::Image => {
                let image = rect.shrink(2.0);
                painter.rect_stroke(image, 2.0, stroke, StrokeKind::Inside);
                painter.circle_filled(
                    egui::pos2(image.right() - 4.0, image.top() + 4.0),
                    1.5,
                    color,
                );
                painter.add(egui::Shape::line(
                    vec![
                        egui::pos2(image.left() + 2.0, image.bottom() - 3.0),
                        egui::pos2(image.center().x - 1.0, image.center().y),
                        egui::pos2(image.center().x + 2.0, image.bottom() - 5.0),
                        egui::pos2(image.right() - 2.0, image.bottom() - 2.0),
                    ],
                    stroke,
                ));
            }
            Self::Network => {
                let first = egui::pos2(rect.left() + 3.0, rect.center().y);
                let upper = egui::pos2(rect.right() - 3.0, rect.top() + 3.5);
                let lower = egui::pos2(rect.right() - 3.0, rect.bottom() - 3.5);
                painter.line_segment([first, upper], stroke);
                painter.line_segment([first, lower], stroke);
                for center in [first, upper, lower] {
                    painter.circle_filled(center, 2.4, color);
                }
            }
            Self::List => {
                for offset in [-4.0, 0.0, 4.0] {
                    let y = rect.center().y + offset;
                    painter.circle_filled(egui::pos2(rect.left() + 3.0, y), 1.3, color);
                    painter.line_segment(
                        [
                            egui::pos2(rect.left() + 6.0, y),
                            egui::pos2(rect.right() - 1.0, y),
                        ],
                        stroke,
                    );
                }
            }
            Self::Console => {
                let console = rect.shrink(1.5);
                painter.rect_stroke(console, 1.5, stroke, StrokeKind::Inside);
                let prompt_x = console.left() + 3.0;
                let prompt_y = console.center().y;
                painter.line_segment(
                    [
                        egui::pos2(prompt_x, prompt_y - 2.5),
                        egui::pos2(prompt_x + 2.5, prompt_y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(prompt_x + 2.5, prompt_y),
                        egui::pos2(prompt_x, prompt_y + 2.5),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(prompt_x + 4.5, prompt_y + 2.5),
                        egui::pos2(console.right() - 2.0, prompt_y + 2.5),
                    ],
                    stroke,
                );
            }
            Self::Eye => {
                let center = rect.center();
                let left = egui::pos2(rect.left() + 1.0, center.y);
                let right = egui::pos2(rect.right() - 1.0, center.y);
                painter.add(egui::Shape::line(
                    vec![
                        left,
                        egui::pos2(center.x - 3.5, center.y - 4.0),
                        egui::pos2(center.x + 3.5, center.y - 4.0),
                        right,
                        egui::pos2(center.x + 3.5, center.y + 4.0),
                        egui::pos2(center.x - 3.5, center.y + 4.0),
                        left,
                    ],
                    stroke,
                ));
                painter.circle_filled(center, 2.2, color);
            }
            Self::Chip => {
                let chip = Rect::from_center_size(rect.center(), egui::vec2(9.0, 9.0));
                painter.rect_stroke(chip, 1.5, stroke, StrokeKind::Inside);
                for offset in [-3.0, 0.0, 3.0] {
                    painter.line_segment(
                        [
                            egui::pos2(chip.left() - 2.5, chip.center().y + offset),
                            egui::pos2(chip.left(), chip.center().y + offset),
                        ],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(chip.right(), chip.center().y + offset),
                            egui::pos2(chip.right() + 2.5, chip.center().y + offset),
                        ],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(chip.center().x + offset, chip.top() - 2.5),
                            egui::pos2(chip.center().x + offset, chip.top()),
                        ],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(chip.center().x + offset, chip.bottom()),
                            egui::pos2(chip.center().x + offset, chip.bottom() + 2.5),
                        ],
                        stroke,
                    );
                }
                painter.rect_filled(chip.shrink(2.5), 0.5, color);
            }
            Self::Target => {
                let center = rect.center();
                painter.circle_stroke(center, 4.2, stroke);
                painter.circle_filled(center, 1.7, color);
                painter.line_segment(
                    [
                        egui::pos2(center.x, rect.top() + 1.0),
                        egui::pos2(center.x, center.y - 5.5),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(center.x, center.y + 5.5),
                        egui::pos2(center.x, rect.bottom() - 1.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(rect.left() + 1.0, center.y),
                        egui::pos2(center.x - 5.5, center.y),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(center.x + 5.5, center.y),
                        egui::pos2(rect.right() - 1.0, center.y),
                    ],
                    stroke,
                );
            }
            Self::Table => {
                let table = rect.shrink(1.5);
                painter.rect_stroke(table, 1.5, stroke, StrokeKind::Inside);
                for fraction in [1.0 / 3.0, 2.0 / 3.0] {
                    let x = egui::lerp(table.left()..=table.right(), fraction);
                    let y = egui::lerp(table.top()..=table.bottom(), fraction);
                    painter.line_segment(
                        [egui::pos2(x, table.top()), egui::pos2(x, table.bottom())],
                        stroke,
                    );
                    painter.line_segment(
                        [egui::pos2(table.left(), y), egui::pos2(table.right(), y)],
                        stroke,
                    );
                }
            }
            Self::Reset => {
                let center = rect.center();
                let radius = 5.5;
                let points = (0..=12)
                    .map(|step| {
                        let angle = -2.5 + step as f32 * 4.5 / 12.0;
                        center + egui::vec2(angle.cos(), angle.sin()) * radius
                    })
                    .collect();
                painter.add(egui::Shape::line(points, stroke));
                let tip = center + egui::vec2((-2.5_f32).cos(), (-2.5_f32).sin()) * radius;
                painter.line_segment([tip, tip + egui::vec2(0.5, -3.5)], stroke);
                painter.line_segment([tip, tip + egui::vec2(3.4, -0.8)], stroke);
            }
        }
    }

    /// Adds an icon-and-label row suitable for an egui popup menu.
    ///
    /// # Parameters
    /// - `ui`: Menu UI receiving the widget.
    /// - `label`: Text rendered beside the icon.
    pub fn menu_item(self, ui: &mut Ui, label: &str) -> egui::Response {
        ui.add(PanelIconMenuItem { icon: self, label })
    }
}

struct PanelIconMenuItem<'a> {
    icon: PanelIcon,
    label: &'a str,
}

impl egui::Widget for PanelIconMenuItem<'_> {
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let width = ui.available_width().max(150.0);
        let response = ui.add_sized(
            [width, 24.0],
            egui::Button::new(egui::RichText::new(self.label).color(Color32::TRANSPARENT)),
        );
        let color = ui.visuals().widgets.style(&response).fg_stroke.color;
        self.icon.paint(
            ui,
            Rect::from_center_size(
                egui::pos2(response.rect.left() + 14.0, response.rect.center().y),
                egui::vec2(16.0, 16.0),
            ),
            color,
        );
        ui.painter().text(
            egui::pos2(response.rect.left() + 28.0, response.rect.center().y),
            egui::Align2::LEFT_CENTER,
            self.label,
            egui::TextStyle::Button.resolve(ui.style()),
            color,
        );
        response
    }
}
