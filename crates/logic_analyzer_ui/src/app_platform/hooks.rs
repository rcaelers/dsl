use std::path::{Path, PathBuf};

use node_graph::api::NodeId;

use super::confirmation_dialog::{
    ACCENT_COLOR, ConfirmationChoice, DESTRUCTIVE_BUTTON_COLOR, DESTRUCTIVE_TEXT_COLOR,
    DestructiveConfirmation, show_destructive_confirmation, show_prominent_modal,
};
use super::state::{FileCommand, GuardedAction};
use crate::app::App;
use crate::capture_provider::{CaptureDataProvider, PreparedCaptureProvider};
use crate::host_service::{HostCommand, OpenDialog, SaveDialog};
use crate::live_capture::{CaptureCoordinatorContract, CaptureRawExportFormat};
use crate::panel_presentation::{LOGIC_ANALYZER_PANEL_ICON, NODE_GRAPH_PANEL_ICON};
use crate::product::APPLICATION_NAME;

impl App {
    fn can_replace_graph(&mut self) -> bool {
        if self.capture_analysis.coordinator().is_active() || self.is_capture_analysis_active() {
            self.toasts
                .error("Wait for live capture analysis before replacing the graph");
            false
        } else {
            true
        }
    }

    pub(crate) fn platform_load_startup_file(&mut self, file: Option<&std::path::Path>) {
        if let Some(file) = file {
            self.load_file(file.to_owned());
        }
    }

    pub(crate) fn platform_raw_input_hook(
        &mut self,
        _ctx: &egui::Context,
        _raw_input: &mut egui::RawInput,
    ) {
    }

    pub(crate) fn platform_logic(&mut self, ctx: &egui::Context) {
        for command in self.host_service.take_commands() {
            let command = match command {
                HostCommand::About => {
                    self.about.open();
                    continue;
                }
                HostCommand::Preferences => {
                    self.preferences.open();
                    continue;
                }
                HostCommand::Run => {
                    self.run_command();
                    continue;
                }
                HostCommand::Stop => {
                    self.stop_command();
                    continue;
                }
                HostCommand::ClearDerivedCaches => {
                    self.request_clear_all_derived_caches();
                    continue;
                }
                HostCommand::ShowLogicAnalyzer => {
                    self.show_primary_panel("logic_analyzer");
                    continue;
                }
                HostCommand::ShowNodeGraph => {
                    self.show_primary_panel("node_graph");
                    continue;
                }
                HostCommand::ShowLog => {
                    self.show_auxiliary_panel("log");
                    continue;
                }
                HostCommand::ShowMemory => {
                    self.show_auxiliary_panel("memory");
                    continue;
                }
                HostCommand::ShowWatches => {
                    self.show_auxiliary_panel("watches");
                    continue;
                }
                HostCommand::ShowTriggers => {
                    self.show_auxiliary_panel("triggers");
                    continue;
                }
                HostCommand::ShowDecoder => {
                    self.show_auxiliary_panel("decoder");
                    continue;
                }
                HostCommand::ResetLaneHeights => {
                    self.reset_viewer_lane_heights();
                    continue;
                }
                HostCommand::ResetLayout => {
                    self.reset_panel_layout();
                    continue;
                }
                HostCommand::New => FileCommand::New,
                HostCommand::Load => FileCommand::Load,
                HostCommand::LoadPath(path) => FileCommand::LoadPath(path),
                HostCommand::ClearRecent => FileCommand::ClearRecent,
                HostCommand::Save => FileCommand::Save,
                HostCommand::SaveAs => FileCommand::SaveAs,
                HostCommand::SaveCaptureData => FileCommand::SaveCaptureData,
                HostCommand::Quit => FileCommand::Quit,
            };
            self.execute_file_command(command, ctx);
        }

        self.poll_derived_cache_clear(ctx);

        if self.host_ui_capabilities.viewport_close_guard {
            let close_requested = ctx.input(|input| input.viewport().close_requested());
            if !self.platform.close_allowed() && close_requested {
                if self.has_unsaved_changes() {
                    self.platform.request_guarded_action(GuardedAction::Quit);
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                } else {
                    self.platform.allow_close();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    pub(crate) fn platform_save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Err(error) = self.sync_panel_layout_setting() {
            self.toasts
                .error(format!("Could not update the graph panel layout: {error}"));
        }
        self.platform.save(storage, self.node_graph.ui_prefs());
    }

    pub(crate) fn platform_before_ui(&mut self, ui: &mut egui::Ui) {
        if !self.host_ui_capabilities.system_menu_bar {
            self.show_menu_bar(ui);
        }
    }

    pub(crate) fn platform_sync_capture(&mut self) -> bool {
        if self.logic_analyzer.has_growing_capture() {
            return false;
        }
        let readiness = self
            .graph_run
            .run()
            .map(|run| run.source_readiness().clone());
        let mut provider = PreparedCaptureProvider::new(
            self.graph_run.service_mut(),
            self.node_graph.graph(),
            readiness,
        );
        let poll = provider.poll();
        self.apply_capture_provider_poll(poll)
    }

    pub(crate) fn platform_restore_graph_capture(&mut self) {
        self.graph_run.service_mut().reset_prepared_capture();
    }

    pub(crate) fn platform_before_graph(&mut self) {
        self.node_graph
            .set_derived_cache_nodes(self.platform.derived_cache_nodes().iter().copied());
    }

    pub(crate) fn platform_after_graph(&mut self) {
        if let Some(node_id) = self.node_graph.take_clear_derived_cache_request() {
            self.clear_node_derived_cache(node_id);
        }
    }

    pub(crate) fn platform_after_ui(&mut self, ctx: &egui::Context) {
        self.show_guarded_action_dialog(ctx);
        self.show_clear_recent_dialog(ctx);
        self.show_clear_derived_caches_dialog(ctx);
    }
    fn load_file(&mut self, path: PathBuf) {
        if !self.can_replace_graph() {
            return;
        }
        match self.host_service.load_graph(&path) {
            Ok(graph) => {
                self.clear_derived_data_presentations();
                self.capture_analysis.coordinator_mut().clear_completed();
                self.graph_run.clear_run_message();
                self.error_badges.clear();
                self.apply_graph_document(graph);
                self.platform.set_current_file(path.clone());
                self.mark_graph_saved();
                self.push_recent_file(path.clone());
                self.refresh_derived_cache_nodes();
                let name = self.host_service.document_display_name(&path);
                self.toasts.info(format!("Loaded {name}"));
            }
            Err(error) => self.toasts.error(error.to_string()),
        }
    }

    /// Inserts `path` at the front of the MRU list, deduping and capping at
    /// `MAX_RECENT_FILES` (Phase 5.1).
    fn push_recent_file(&mut self, path: PathBuf) {
        self.platform.push_recent_file(path);
        self.host_service
            .publish_recent_files(self.platform.recent_files());
    }

    /// Resets to a fresh, empty graph — File → New (Phase 5.1). Assumes the
    /// unsaved-changes guard has already been resolved by the caller.
    fn do_new(&mut self) {
        if !self.can_replace_graph() {
            return;
        }
        self.clear_derived_data_presentations();
        self.capture_analysis.coordinator_mut().clear_completed();
        self.graph_run.clear_run_message();
        self.error_badges.clear();
        self.node_graph.new_graph();
        self.graph_run.replace_cached_preview_revision(None);
        self.restore_sampling_overlay_setting();
        self.restore_viewer_lane_order_setting();
        self.restore_viewer_lane_height_setting();
        self.restore_timeline_cursor_setting();
        self.restore_panel_layout_setting();
        self.platform.clear_derived_cache_nodes();
        self.platform.clear_current_file();
        self.mark_graph_saved();
        self.toasts.info("New graph");
    }

    /// Requests File → New, guarding on unsaved changes the same way
    /// `request_quit` does.
    fn request_new(&mut self) {
        if self.has_unsaved_changes() {
            self.platform.request_guarded_action(GuardedAction::New);
        } else {
            self.do_new();
        }
    }

    /// Requests loading `path` (e.g. from Open Recent), guarding on unsaved
    /// changes the same way `request_quit` does.
    fn request_load_path(&mut self, path: PathBuf) {
        if self.has_unsaved_changes() {
            self.platform
                .request_guarded_action(GuardedAction::LoadPath(path));
        } else {
            self.load_file(path);
        }
    }

    fn choose_and_load_file(&mut self) {
        let initial_directory = self
            .platform
            .current_file()
            .and_then(|path| path.parent())
            .map(Path::to_owned);
        let path = self.host_service.choose_open_file(OpenDialog {
            title: "Open graph",
            filter_label: "Graph JSON",
            extensions: &["json"],
            initial_directory: initial_directory.as_deref(),
        });
        if let Some(path) = path {
            self.load_file(path);
        }
    }

    fn save_file(&mut self) -> bool {
        let Some(path) = self.platform.current_file().map(Path::to_owned) else {
            return self.save_file_as();
        };
        self.save_to_file(path)
    }

    fn save_file_as(&mut self) -> bool {
        let initial_directory = self
            .platform
            .current_file()
            .and_then(|path| path.parent())
            .map(Path::to_owned);
        let default_file_name = self
            .platform
            .current_file()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("pipeline.json")
            .to_owned();
        let path = self.host_service.choose_save_file(SaveDialog {
            title: "Save graph as",
            default_file_name: &default_file_name,
            filter_label: "Graph JSON",
            extensions: &["json"],
            initial_directory: initial_directory.as_deref(),
        });
        let Some(path) = path else {
            return false;
        };
        self.save_to_file(path)
    }

    fn save_to_file(&mut self, path: PathBuf) -> bool {
        if let Err(error) = self.sync_panel_layout_setting() {
            self.toasts
                .error(format!("Could not save the panel layout: {error}"));
            return false;
        }
        self.synchronize_payload_subscription_manifest(false);
        let graph = match self.node_graph.snapshot_value() {
            Ok(graph) => graph,
            Err(error) => {
                self.toasts.error(error.to_string());
                return false;
            }
        };
        match self.host_service.save_graph(&path, &graph) {
            Ok(()) => {
                self.platform.set_current_file(path.clone());
                self.mark_graph_saved();
                self.push_recent_file(path.clone());
                let name = self.host_service.document_display_name(&path);
                self.toasts.info(format!("Saved {name}"));
                true
            }
            Err(error) => {
                self.toasts.error(error.to_string());
                false
            }
        }
    }

    fn choose_and_save_capture_data(&mut self) {
        let format = CaptureRawExportFormat::Portable;
        let descriptor = format.descriptor();
        let initial_directory = self
            .platform
            .current_file()
            .and_then(|path| path.parent())
            .map(Path::to_owned);
        let Some(mut path) = self.host_service.choose_save_file(SaveDialog {
            title: descriptor.dialog_title,
            default_file_name: descriptor.default_file_name,
            filter_label: descriptor.label,
            extensions: &[descriptor.extension],
            initial_directory: initial_directory.as_deref(),
        }) else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension(descriptor.extension);
        }
        if let Err(error) = self
            .capture_analysis
            .coordinator_mut()
            .start_export_current(format, path)
        {
            self.toasts.error(error.to_string());
        }
    }

    fn mark_graph_saved(&mut self) {
        if let Err(error) = self.sync_panel_layout_setting() {
            self.toasts
                .error(format!("Could not save the panel layout: {error}"));
            return;
        }
        self.synchronize_payload_subscription_manifest(false);
        match self.node_graph.snapshot_value() {
            Ok(graph) => self.platform.mark_saved_graph(graph),
            Err(error) => self.toasts.error(error.to_string()),
        }
    }

    fn has_unsaved_changes(&mut self) -> bool {
        if self.sync_panel_layout_setting().is_err() {
            return true;
        }
        self.synchronize_payload_subscription_manifest(false);
        self.node_graph
            .snapshot_value()
            .map_or(true, |graph| !self.platform.is_saved_graph(&graph))
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        if self.has_unsaved_changes() {
            self.platform.request_guarded_action(GuardedAction::Quit);
        } else {
            self.platform.allow_close();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn execute_file_command(&mut self, command: FileCommand, ctx: &egui::Context) {
        match command {
            FileCommand::New => self.request_new(),
            FileCommand::Load => self.choose_and_load_file(),
            FileCommand::LoadPath(path) => self.request_load_path(path),
            FileCommand::ClearRecent => self.platform.request_clear_recent_confirmation(),
            FileCommand::Save => {
                self.save_file();
            }
            FileCommand::SaveAs => {
                self.save_file_as();
            }
            FileCommand::SaveCaptureData => self.choose_and_save_capture_data(),
            FileCommand::Quit => self.request_quit(ctx),
        }
    }

    /// Resolves whatever `pending_guarded_action` (quit/new/load-over-dirty)
    /// is outstanding — Save/Don't Save/Cancel, same dialog for all three
    /// (Phase 5.1).
    fn show_guarded_action_dialog(&mut self, ctx: &egui::Context) {
        let continuation = match self.platform.guarded_action() {
            Some(GuardedAction::Quit) => "quitting",
            Some(GuardedAction::New) => "creating a new graph",
            Some(GuardedAction::LoadPath(_)) => "opening another graph",
            None => return,
        };

        enum DialogChoice {
            Save,
            Discard,
            Cancel,
        }

        let mut choice = None;
        let modal = show_prominent_modal(ctx, "unsaved-graph-changes", |ui| {
            ui.label(
                egui::RichText::new("Unsaved changes")
                    .size(26.0)
                    .strong()
                    .color(ACCENT_COLOR),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "Your graph has changes that have not been saved. Save before {continuation}?"
                ))
                .size(16.0),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new("Choosing Don’t Save permanently discards those changes.")
                    .color(DESTRUCTIVE_TEXT_COLOR),
            );
            ui.add_space(20.0);

            ui.horizontal(|ui| {
                if ui
                    .add_sized([108.0, 32.0], egui::Button::new("Keep Editing"))
                    .clicked()
                {
                    choice = Some(DialogChoice::Cancel);
                }
                let remaining_width = ui.available_width();
                ui.allocate_ui_with_layout(
                    egui::Vec2::new(remaining_width, 32.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui
                            .add_sized(
                                [132.0, 32.0],
                                egui::Button::new("Save Changes")
                                    .fill(ui.visuals().selection.bg_fill),
                            )
                            .clicked()
                        {
                            choice = Some(DialogChoice::Save);
                        }
                        ui.add_space(8.0);
                        if ui
                            .add_sized(
                                [112.0, 32.0],
                                egui::Button::new("Don’t Save").fill(DESTRUCTIVE_BUTTON_COLOR),
                            )
                            .clicked()
                        {
                            choice = Some(DialogChoice::Discard);
                        }
                    },
                );
            });
        });

        if choice.is_none() && modal.should_close() {
            choice = Some(DialogChoice::Cancel);
        }

        match choice {
            // Save can itself open a blocking Save As dialog and be
            // cancelled — leave `pending_guarded_action` set so this dialog
            // simply reopens next frame rather than silently dropping the
            // action.
            Some(DialogChoice::Save) if self.save_file() => self.complete_guarded_action(ctx),
            Some(DialogChoice::Discard) => self.complete_guarded_action(ctx),
            Some(DialogChoice::Cancel) => self.platform.cancel_guarded_action(),
            _ => {}
        }
    }

    fn complete_guarded_action(&mut self, ctx: &egui::Context) {
        let Some(action) = self.platform.take_guarded_action() else {
            return;
        };
        match action {
            GuardedAction::Quit => {
                self.platform.allow_close();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            GuardedAction::New => self.do_new(),
            GuardedAction::LoadPath(path) => self.load_file(path),
        }
    }

    /// Resolves the "Clear the recent files list?" confirmation triggered
    /// by either the egui or native "Clear Recent" menu item.
    fn show_clear_recent_dialog(&mut self, ctx: &egui::Context) {
        if !self.platform.clear_recent_confirmation_requested() {
            return;
        }

        let choice = show_destructive_confirmation(
            ctx,
            DestructiveConfirmation {
                id: "clear-recent-files",
                title: "Clear recent files?",
                message: "Remove every entry from the recent files list?",
                detail: "This does not delete the graph files themselves.",
                confirm_label: "Clear Recent",
            },
        );

        match choice {
            Some(ConfirmationChoice::Confirm) => {
                self.platform.clear_recent_files();
                self.host_service.publish_recent_files(&[]);
                self.platform.finish_clear_recent_confirmation();
            }
            Some(ConfirmationChoice::Cancel) => {
                self.platform.finish_clear_recent_confirmation();
            }
            None => {}
        }
    }

    fn show_clear_derived_caches_dialog(&mut self, ctx: &egui::Context) {
        if !self.platform.clear_derived_caches_confirmation_requested() {
            return;
        }

        match show_destructive_confirmation(
            ctx,
            DestructiveConfirmation {
                id: "clear-all-derived-data-caches",
                title: "Clear all derived data caches?",
                message: "Cached decoded data for every pipeline will be removed.",
                detail: "This cannot be undone. Required data must be rebuilt by running the pipeline again.",
                confirm_label: "Clear All",
            },
        ) {
            Some(ConfirmationChoice::Confirm) => {
                self.platform.finish_clear_derived_caches_confirmation();
                self.clear_all_derived_caches();
            }
            Some(ConfirmationChoice::Cancel) => {
                self.platform.finish_clear_derived_caches_confirmation();
            }
            None => {}
        }
    }

    fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        let shortcut = |action| {
            self.input_bindings
                .shortcut(&["global"], action)
                .unwrap_or_else(|| panic!("missing global.{action} input binding"))
        };
        let new_shortcut = shortcut("new");
        let load_shortcut = shortcut("open");
        let save_shortcut = shortcut("save");
        let save_as_shortcut = shortcut("save_as");
        let quit_shortcut = shortcut("quit");
        let run_shortcut = shortcut("run");
        let stop_shortcut = shortcut("stop");
        let mut command = if !self.host_ui_capabilities.direct_document_access {
            None
        } else if ui.input_mut(|input| input.consume_shortcut(&new_shortcut)) {
            Some(FileCommand::New)
        } else if ui.input_mut(|input| input.consume_shortcut(&load_shortcut)) {
            Some(FileCommand::Load)
        } else if ui.input_mut(|input| input.consume_shortcut(&save_as_shortcut)) {
            Some(FileCommand::SaveAs)
        } else if ui.input_mut(|input| input.consume_shortcut(&save_shortcut)) {
            Some(FileCommand::Save)
        } else if ui.input_mut(|input| input.consume_shortcut(&quit_shortcut)) {
            Some(FileCommand::Quit)
        } else {
            None
        };
        // Not routed through `command`/`execute_file_command` like the File
        // items above — Run/Stop are self-contained and idempotent
        // (`run_command`/`stop_command` no-op when they don't apply), so
        // there's nothing to defer.
        if ui.input_mut(|input| input.consume_shortcut(&run_shortcut)) {
            self.run_command();
        } else if ui.input_mut(|input| input.consume_shortcut(&stop_shortcut)) {
            self.stop_command();
        }

        egui::MenuBar::new().ui(ui, |ui| {
            if self.host_ui_capabilities.direct_document_access {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(
                            egui::Button::new("New")
                                .shortcut_text(ui.ctx().format_shortcut(&new_shortcut)),
                        )
                        .clicked()
                    {
                        command = Some(FileCommand::New);
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::new("Open...")
                                .shortcut_text(ui.ctx().format_shortcut(&load_shortcut)),
                        )
                        .clicked()
                    {
                        command = Some(FileCommand::Load);
                        ui.close();
                    }
                    ui.menu_button("Open Recent", |ui| {
                        let existing: Vec<PathBuf> = self
                            .recent_files()
                            .iter()
                            .filter(|path| self.host_service.document_exists(path))
                            .cloned()
                            .collect();
                        if existing.is_empty() {
                            ui.weak("No recent files");
                        } else {
                            for path in &existing {
                                let label = path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("?");
                                if ui.button(label).clicked() {
                                    command = Some(FileCommand::LoadPath(path.clone()));
                                    ui.close();
                                }
                            }
                        }
                        ui.separator();
                        if ui
                            .add_enabled(!existing.is_empty(), egui::Button::new("Clear Recent"))
                            .clicked()
                        {
                            command = Some(FileCommand::ClearRecent);
                            ui.close();
                        }
                    });
                    if ui
                        .add(
                            egui::Button::new("Save")
                                .shortcut_text(ui.ctx().format_shortcut(&save_shortcut)),
                        )
                        .clicked()
                    {
                        command = Some(FileCommand::Save);
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::new("Save As...")
                                .shortcut_text(ui.ctx().format_shortcut(&save_as_shortcut)),
                        )
                        .clicked()
                    {
                        command = Some(FileCommand::SaveAs);
                        ui.close();
                    }
                    ui.separator();
                    let can_save_capture = self
                        .capture_analysis
                        .coordinator()
                        .current_session_id()
                        .is_some()
                        && !self.capture_analysis.coordinator().is_active()
                        && self
                            .capture_analysis
                            .coordinator()
                            .export_status()
                            .is_none();
                    if ui
                        .add_enabled(can_save_capture, egui::Button::new("Save Capture Data..."))
                        .on_disabled_hover_text("Finish a capture before saving its data")
                        .clicked()
                    {
                        command = Some(FileCommand::SaveCaptureData);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Preferences...").clicked() {
                        self.preferences.open();
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add(
                            egui::Button::new("Quit")
                                .shortcut_text(ui.ctx().format_shortcut(&quit_shortcut)),
                        )
                        .clicked()
                    {
                        command = Some(FileCommand::Quit);
                        ui.close();
                    }
                });
            }
            ui.menu_button("View", |ui| {
                for (label, content_id, icon) in [
                    (
                        "Logic Analyzer",
                        "logic_analyzer",
                        LOGIC_ANALYZER_PANEL_ICON.panel_icon(),
                    ),
                    (
                        "Node Graph",
                        "node_graph",
                        NODE_GRAPH_PANEL_ICON.panel_icon(),
                    ),
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
                ui.separator();
                if ui
                    .add_enabled(
                        !self.is_running() && self.graph_run.cache_clear_task().is_none(),
                        egui::Button::new("Clear All Derived Data Caches..."),
                    )
                    .clicked()
                {
                    self.request_clear_all_derived_caches();
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

        if let Some(command) = command {
            self.execute_file_command(command, ui.ctx());
        }
    }

    fn can_clear_derived_caches(&mut self) -> bool {
        if self.is_running() {
            self.toasts
                .error("Stop the pipeline before clearing derived data caches");
            false
        } else if self.graph_run.cache_clear_task().is_some() {
            self.toasts
                .info("Derived data caches are already being cleared");
            false
        } else {
            true
        }
    }

    fn release_derived_data_handles(&mut self) {
        self.clear_derived_data_presentations();
    }

    fn refresh_derived_cache_nodes(&mut self) {
        let nodes = self
            .graph_run
            .service()
            .derived_cache_configs_by_node(self.node_graph.graph())
            .map(|inventory| {
                inventory
                    .into_keys()
                    .filter(|id| self.node_graph.graph().nodes.contains_key(id))
                    .collect()
            })
            .unwrap_or_default();
        self.platform.set_derived_cache_nodes(nodes);
    }

    fn clear_node_derived_cache(&mut self, node_id: NodeId) {
        if !self.can_clear_derived_caches() {
            return;
        }
        let node_name = self
            .node_graph
            .graph()
            .nodes
            .get(&node_id)
            .map(|node| node.title.clone())
            .unwrap_or_else(|| "node".to_owned());
        let configs = match self
            .graph_run
            .service()
            .derived_cache_configs_by_node(self.node_graph.graph())
        {
            Ok(mut inventory) => inventory.remove(&node_id).unwrap_or_default(),
            Err(errors) => {
                let message = errors
                    .first()
                    .map(|error| error.message.as_str())
                    .unwrap_or("graph could not be compiled");
                self.toasts
                    .error(format!("Cannot determine cache: {message}"));
                return;
            }
        };
        if configs.is_empty() {
            self.toasts
                .info(format!("No derived data cache found for {node_name}"));
            return;
        }

        self.release_derived_data_handles();
        let mut removed_entries = 0usize;
        let mut removed_bytes = 0u64;
        for config in &configs {
            match self.graph_run.service().clear_derived_cache_entry(config) {
                Ok(stats) => {
                    removed_entries += stats.removed_entries;
                    removed_bytes = removed_bytes.saturating_add(stats.removed_bytes);
                }
                Err(error) => {
                    self.toasts.error(format!("Failed to clear cache: {error}"));
                    return;
                }
            }
        }
        if removed_entries == 0 {
            self.toasts
                .info(format!("No derived data cache found for {node_name}"));
        } else {
            self.toasts.info(format!(
                "Cleared {removed_entries} derived cache entr{} for {node_name} ({removed_bytes} bytes)",
                if removed_entries == 1 { "y" } else { "ies" }
            ));
        }
    }

    fn request_clear_all_derived_caches(&mut self) {
        if self.can_clear_derived_caches() {
            self.platform.request_clear_derived_caches_confirmation();
        }
    }

    fn clear_all_derived_caches(&mut self) {
        if !self.can_clear_derived_caches() {
            return;
        }
        self.release_derived_data_handles();
        match self.graph_run.service().start_clear_derived_caches() {
            Ok(task) => {
                self.graph_run
                    .set_cached_preview_revision(self.node_graph.graph().semantic_revision());
                self.graph_run.install_cache_clear_task(task);
                self.toasts.info("Clearing derived data caches…");
            }
            Err(error) => {
                self.graph_run.clear_cached_preview_revision();
                self.toasts
                    .error(format!("Failed to start clearing caches: {error}"));
            }
        }
    }

    fn poll_derived_cache_clear(&mut self, ctx: &egui::Context) {
        const COOPERATIVE_ARTIFACT_BUDGET: usize = 16;

        let Some(task) = self.graph_run.cache_clear_task_mut() else {
            return;
        };
        let Some(result) = task.poll(COOPERATIVE_ARTIFACT_BUDGET) else {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        };
        self.graph_run.clear_cache_clear_task();
        self.platform.clear_derived_cache_nodes();
        match result {
            Ok(stats) if stats.removed_entries == 0 && stats.removed_bytes == 0 => {
                self.toasts.info("No derived data caches found");
            }
            Ok(stats) => self.toasts.info(format!(
                "Cleared {} derived cache entr{} ({} bytes)",
                stats.removed_entries,
                if stats.removed_entries == 1 {
                    "y"
                } else {
                    "ies"
                },
                stats.removed_bytes
            )),
            Err(error) => {
                self.graph_run.clear_cached_preview_revision();
                self.toasts
                    .error(format!("Failed to clear caches: {error}"));
            }
        }
    }
}
