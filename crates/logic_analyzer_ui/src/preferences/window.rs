use crate::node_catalog_service::NodeCatalogService;

pub(crate) struct PreferencesWindow {
    open: bool,
}

impl PreferencesWindow {
    pub(crate) fn new() -> Self {
        Self { open: false }
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        catalogs: &mut [Box<dyn NodeCatalogService>],
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
                        let _ = ui.selectable_label(true, "External Decoders");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
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
