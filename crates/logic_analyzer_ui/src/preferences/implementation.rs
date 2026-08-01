use logic_analyzer_graph_api::node::DirectoryNodeCatalog;

use crate::host_service::HostService;

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
        catalogs: &mut [Box<dyn DirectoryNodeCatalog>],
        host_service: &mut dyn HostService,
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
                            draw_catalog(ui, catalog.as_mut(), host_service);
                            ui.add_space(12.0);
                        }
                    });
                });
            });
    }
}

fn draw_catalog(
    ui: &mut egui::Ui,
    catalog: &mut dyn DirectoryNodeCatalog,
    host_service: &mut dyn HostService,
) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(catalog.title())
            .default_open(true)
            .show(ui, |ui| {
                let mut directories = catalog.directories();
                let mut remove = None;
                for (index, directory) in directories.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            [ui.available_width() - 38.0, 24.0],
                            egui::Label::new(directory.display().to_string())
                                .truncate()
                                .sense(egui::Sense::hover()),
                        )
                        .on_hover_text(directory.display().to_string());
                        if ui.button("−").on_hover_text("Remove directory").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    directories.remove(index);
                    catalog.set_directories(directories);
                }
                ui.horizontal(|ui| {
                    if ui.button("+ Add Directory").clicked()
                        && let Some(directory) = host_service.choose_directory()
                    {
                        let mut directories = catalog.directories();
                        if !directories.contains(&directory) {
                            directories.push(directory);
                            catalog.set_directories(directories);
                        }
                    }
                    if ui.button("Rescan").clicked() {
                        catalog.rescan();
                    }
                    let status = catalog.status();
                    if status.scanning {
                        ui.spinner();
                        ui.label("Scanning in background…");
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    } else {
                        ui.label(format!("{} decoders found", status.discovered));
                    }
                    for diagnostic in status.diagnostics.iter().take(4) {
                        ui.colored_label(egui::Color32::from_rgb(220, 110, 90), diagnostic);
                    }
                });
            });
    });
}
