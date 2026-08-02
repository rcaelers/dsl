use logic_analyzer_graph_compiler as compiler;

use crate::app::App;
use crate::product::APPLICATION_NAME;

impl App {
    pub(crate) fn platform_load_startup_file(&mut self, _file: Option<&std::path::Path>) {}

    pub(crate) fn platform_raw_input_hook(
        &mut self,
        _ctx: &egui::Context,
        _raw_input: &mut egui::RawInput,
    ) {
    }

    pub(crate) fn platform_logic(&mut self, _ctx: &egui::Context) {}

    pub(crate) fn platform_save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Err(error) = self.sync_panel_layout_setting() {
            self.toasts
                .error(format!("Could not update the graph panel layout: {error}"));
        }
        let state = super::PersistedUiState::capture(self.node_graph.ui_prefs());
        eframe::set_value(storage, eframe::APP_KEY, &state);
    }

    pub(crate) fn platform_before_ui(&mut self, ui: &mut egui::Ui) {
        let shortcut = |action| {
            self.input_bindings
                .shortcut(&["global"], action)
                .unwrap_or_else(|| panic!("missing global.{action} input binding"))
        };
        let run_shortcut = shortcut("run");
        let stop_shortcut = shortcut("stop");
        let mut demo_to_load = None;

        if ui.input_mut(|input| input.consume_shortcut(&run_shortcut)) {
            self.run_command();
        } else if ui.input_mut(|input| input.consume_shortcut(&stop_shortcut)) {
            self.stop_command();
        }

        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("View", |ui| {
                for (label, content_id, icon) in [
                    (
                        "Logic Analyzer",
                        "logic_analyzer",
                        panel_layout::PanelIcon::Waveform,
                    ),
                    ("Node Graph", "node_graph", panel_layout::PanelIcon::Network),
                ] {
                    if icon.menu_item(ui, label).clicked() {
                        self.show_primary_panel(content_id);
                        ui.close();
                    }
                }
                ui.separator();
                for (label, content_id, icon) in self.available_auxiliary_panels() {
                    if icon.menu_item(ui, &label).clicked() {
                        self.show_auxiliary_panel(&content_id);
                        ui.close();
                    }
                }
                ui.separator();
                if panel_layout::PanelIcon::Reset
                    .menu_item(ui, "Reset Lane Heights")
                    .clicked()
                {
                    self.reset_viewer_lane_heights();
                    ui.close();
                }
                if panel_layout::PanelIcon::Reset
                    .menu_item(ui, "Reset Layout")
                    .clicked()
                {
                    self.reset_panel_layout();
                    ui.close();
                }
            });
            if !self.demo_graphs.is_empty() {
                ui.menu_button("Demos", |ui| {
                    for (index, demo) in self.demo_graphs.iter().enumerate() {
                        if ui.button(demo.name()).clicked() {
                            demo_to_load = Some(index);
                            ui.close();
                        }
                    }
                });
            }
            ui.menu_button("Pipeline", |ui| {
                let unavailable = self.run_unavailable_reason();
                let run = ui.add_enabled(
                    unavailable.is_none(),
                    egui::Button::new("Run").shortcut_text(ui.ctx().format_shortcut(&run_shortcut)),
                );
                if let Some(reason) = unavailable {
                    run.clone().on_disabled_hover_text(reason);
                }
                if run.clicked() {
                    self.run_command();
                    ui.close();
                }
                if ui
                    .add(
                        egui::Button::new("Stop")
                            .shortcut_text(ui.ctx().format_shortcut(&stop_shortcut)),
                    )
                    .clicked()
                {
                    self.stop_command();
                    ui.close();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button(format!("About {APPLICATION_NAME}")).clicked() {
                    self.about.open();
                    ui.close();
                }
            });
        });
        if let Some(index) = demo_to_load {
            self.load_demo_graph(index);
        }
    }

    pub(crate) fn platform_sync_capture(&mut self) {
        if self.logic_analyzer.has_growing_capture() {
            return;
        }
        let update = self
            .graph_service
            .synchronize_prepared_capture(self.node_graph.graph());
        match update {
            compiler::SourcePreparationUpdate::Unchanged => {}
            compiler::SourcePreparationUpdate::Preparing => {
                if self.platform.capture_presentation_identity.take().is_some() {
                    self.clear_capture_presentation();
                }
                let progress = self
                    .graph_service
                    .source_preparation_snapshot()
                    .progress
                    .and_then(|progress| {
                        (progress.total > 0)
                            .then(|| progress.completed as f32 / progress.total as f32)
                    });
                self.mark_capture_index_building(progress);
            }
            compiler::SourcePreparationUpdate::Cleared => {
                self.platform.capture_presentation_identity = None;
                self.clear_capture_presentation();
            }
            compiler::SourcePreparationUpdate::Failed(error) => {
                self.platform.capture_presentation_identity = None;
                self.clear_capture_presentation();
                self.toasts
                    .error(format!("Could not prepare capture source: {error}"));
            }
            compiler::SourcePreparationUpdate::Ready(prepared) => {
                self.logic_analyzer
                    .set_visible_capture_channels(prepared.visible_channels);
                self.platform.capture_presentation_identity = Some(prepared.identity.clone());
                match prepared.data {
                    compiler::PreparedCaptureData::Indexed(index) => {
                        self.set_prepared_capture(prepared.identity, index)
                    }
                    compiler::PreparedCaptureData::InMemory {
                        signals,
                        duration_us,
                    } => self.set_capture_preview(signals, duration_us),
                    compiler::PreparedCaptureData::Channels(channels) => {
                        self.set_capture_channel_metadata(prepared.identity, channels)
                    }
                }
            }
        }
        match self.graph_service.source_preparation_status() {
            compiler::SourcePreparationStatus::Ready => self.publish_file_source_ready(),
            compiler::SourcePreparationStatus::Failed(error) => {
                self.publish_file_source_failure(&error)
            }
            compiler::SourcePreparationStatus::Empty
            | compiler::SourcePreparationStatus::Preparing => {}
        }
    }

    pub(crate) fn platform_restore_graph_capture(&mut self) {
        self.platform.capture_presentation_identity = None;
        self.graph_service.reset_prepared_capture();
    }

    pub(crate) fn platform_before_graph(&mut self) {}

    pub(crate) fn platform_after_graph(&mut self) {}

    pub(crate) fn platform_after_ui(&mut self, _ctx: &egui::Context) {}
}
