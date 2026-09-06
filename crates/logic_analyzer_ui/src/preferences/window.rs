use node_graph::ConnectionRouting;

use crate::node_catalog_service::NodeCatalogService;

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreferencesPage {
    NodeGraph,
    ExternalDecoders,
}

pub(crate) struct PreferencesWindow {
    open: bool,
    page: PreferencesPage,
}

impl PreferencesWindow {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            page: PreferencesPage::NodeGraph,
        }
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        catalogs: &mut [Box<dyn NodeCatalogService>],
        connection_routing: &mut ConnectionRouting,
    ) {
        if !self.open {
            return;
        }
        egui::Window::new("Preferences")
            .open(&mut self.open)
            .default_size([880.0, 560.0])
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    ui.set_min_height(470.0);
                    ui.vertical(|ui| {
                        ui.set_width(190.0);
                        ui.heading("Preferences");
                        ui.separator();
                        ui.selectable_value(&mut self.page, PreferencesPage::NodeGraph, "Node Graph");
                        ui.selectable_value(&mut self.page, PreferencesPage::ExternalDecoders, "External Decoders");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.page == PreferencesPage::NodeGraph {
                            ui.heading("Node Graph");
                            ui.add_space(10.0);
                            ui.label("Connection drawing");
                            ui.radio_value(connection_routing, ConnectionRouting::Classic, "Classic curves")
                                .on_hover_text("Direct curves, as on origin/main. Connections may cross nodes; no routing warnings.");
                            ui.radio_value(connection_routing, ConnectionRouting::ObstacleAvoiding, "Obstacle-avoiding")
                                .on_hover_text("Route around nodes, separate independent signals, and combine branches from the same output.");
                            ui.add_space(10.0);
                            ui.weak("Applies immediately and is saved across launches. Node positions and saved connections do not change.");
                            return;
                        }
                        ui.heading("External Decoders");
                        ui.label(
                            "Foreign Python decoders are trusted code and run with application permissions.",
                        );
                        ui.add_space(10.0);
                        for catalog in catalogs {
                            draw_catalog(ui, catalog.as_mut());
                            ui.add_space(12.0);
                        }
                    });
                });
            });
    }
}

fn draw_catalog(ui: &mut egui::Ui, catalog: &mut dyn NodeCatalogService) {
    let snapshot = catalog.snapshot();
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(&snapshot.title)
            .default_open(true)
            .show(ui, |ui| {
                let mut remove = None;
                for (index, directory) in snapshot.directories.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [ui.available_width() - 38.0, 24.0],
                            egui::Label::new(directory)
                                .truncate()
                                .sense(egui::Sense::hover()),
                        )
                        .on_hover_text(directory);
                        if ui.button("−").on_hover_text("Remove directory").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    catalog.remove_directory(index);
                }
                ui.horizontal(|ui| {
                    if ui.button("+ Add Directory").clicked() {
                        catalog.add_directory();
                    }
                    if ui.button("Rescan").clicked() {
                        catalog.rescan();
                    }
                    if snapshot.scanning {
                        ui.spinner();
                        ui.label("Scanning in background…");
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    } else {
                        ui.label(format!("{} decoders found", snapshot.discovered));
                    }
                    for diagnostic in snapshot.diagnostics.iter().take(4) {
                        ui.colored_label(egui::Color32::from_rgb(220, 110, 90), diagnostic);
                    }
                });
            });
    });
}

#[cfg(test)]
mod preferences_tests {
    use super::*;

    #[test]
    fn graph_preferences_switch_connection_drawing_in_both_directions() {
        let ctx = egui::Context::default();
        let mut window = PreferencesWindow::new();
        window.open();
        let mut mode = ConnectionRouting::default();
        let mut frame = |events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 750.0),
                )),
                events,
                ..Default::default()
            });
            window.show(&ctx, &mut [], &mut mode);
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            (output, mode)
        };
        frame(Vec::new());
        let (mut output, _) = frame(Vec::new());
        for (label, expected) in [
            ("Classic curves", ConnectionRouting::Classic),
            ("Obstacle-avoiding", ConnectionRouting::ObstacleAvoiding),
        ] {
            let pos = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == label => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .expect("visible routing preference");
            frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let (next, actual) = frame(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            output = next;
            assert_eq!(actual, expected);
        }
    }
}
