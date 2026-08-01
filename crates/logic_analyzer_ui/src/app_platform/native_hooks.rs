use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use logic_analyzer_graph_compiler as compiler;
use node_graph::NodeId;

use crate::app::App;
use crate::app_platform::{FileCommand, GuardedAction, derived_cache_directory};
#[cfg(target_os = "macos")]
use crate::app_platform::{NativeMenuCommand, notify_recent_files_changed};
use crate::host_service::{OpenDialog, SaveDialog};
use crate::live_capture::{CaptureCoordinatorContract, CaptureRawExportFormat};
use crate::memory_panel::{
    MemoryServiceSnapshot, PersistentCacheSnapshot, PersistentCacheSnapshotState,
    PlatformMemorySnapshot,
};
#[cfg(not(target_os = "macos"))]
use crate::product::APPLICATION_NAME;

impl App {
    pub(crate) fn platform_memory_snapshot(&mut self) -> PlatformMemorySnapshot {
        let decoded = signal_processing::decoded_block_cache_stats();
        let mut snapshot = PlatformMemorySnapshot {
            services: vec![MemoryServiceSnapshot {
                name: "Decoded block cache".to_owned(),
                state: if decoded.entries == 0 {
                    "Empty"
                } else {
                    "Ready"
                }
                .to_owned(),
                detail: format!(
                    "{} block(s) · {} hit(s) · {} miss(es)",
                    decoded.entries, decoded.hits, decoded.misses
                ),
                used_bytes: Some(decoded.memory_bytes as u64),
                budget_bytes: Some(decoded.budget_bytes as u64),
            }],
            persistent_caches: Vec::new(),
        };
        let directory = derived_cache_directory();
        let inventory = match self
            .graph_service
            .derived_cache_configs_by_node(self.node_graph.graph(), &directory)
        {
            Ok(inventory) => inventory,
            Err(errors) => {
                snapshot.services.push(MemoryServiceSnapshot {
                    name: "Persistent derived cache".to_owned(),
                    state: "Unavailable".to_owned(),
                    detail: errors.first().map_or_else(
                        || "Graph cannot be lowered".to_owned(),
                        |error| error.message.clone(),
                    ),
                    used_bytes: None,
                    budget_bytes: None,
                });
                return snapshot;
            }
        };
        let mut entries = BTreeMap::new();
        for (node_id, configs) in inventory {
            let owner = self
                .node_graph
                .graph()
                .nodes
                .get(&node_id)
                .map(|node| node.title.clone());
            for config in configs {
                let (_, owners): &mut (signal_processing::PersistentStoreConfig, BTreeSet<String>) =
                    entries
                        .entry(config.cache_key)
                        .or_insert_with(|| (config.clone(), BTreeSet::new()));
                if let Some(owner) = &owner {
                    owners.insert(owner.clone());
                }
            }
        }
        for (_, (config, owners)) in entries {
            let inspected = self.host_service.inspect_cache_entry(&config);
            let (state, info) = match inspected {
                Ok(Some(info)) => (PersistentCacheSnapshotState::Ready, Some(info)),
                Ok(None) => (PersistentCacheSnapshotState::Missing, None),
                Err(error) => (PersistentCacheSnapshotState::Unreadable(error), None),
            };
            snapshot.persistent_caches.push(PersistentCacheSnapshot {
                cache_key: config.cache_key,
                owners: owners.into_iter().collect(),
                directory: config.directory,
                state,
                total_bytes: info.map(|info| info.total_bytes),
                data_bytes: info.map(|info| info.data_bytes),
                index_bytes: info.map(|info| info.index_bytes),
                items: info.map(|info| info.item_count),
                index_items: info.map(|info| info.index_item_count),
            });
        }
        let ready = snapshot
            .persistent_caches
            .iter()
            .filter(|entry| entry.state == PersistentCacheSnapshotState::Ready)
            .count();
        let bytes = snapshot
            .persistent_caches
            .iter()
            .filter_map(|entry| entry.total_bytes)
            .fold(0u64, u64::saturating_add);
        snapshot.services.push(MemoryServiceSnapshot {
            name: "Persistent derived cache".to_owned(),
            state: if snapshot.persistent_caches.is_empty() {
                "Empty"
            } else if ready == 0 {
                "Missing"
            } else {
                "Ready"
            }
            .to_owned(),
            detail: format!(
                "{ready} ready of {} selected graph entr{}",
                snapshot.persistent_caches.len(),
                if snapshot.persistent_caches.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            ),
            used_bytes: Some(bytes),
            budget_bytes: None,
        });
        snapshot
    }

    pub(crate) fn platform_clear_capture_caches(
        &mut self,
        configs: &[signal_processing::PersistentStoreConfig],
    ) -> Result<(), String> {
        for config in configs {
            self.host_service.clear_cache_entry(config)?;
        }
        Ok(())
    }

    fn can_replace_graph(&mut self) -> bool {
        if self.capture.is_active() || self.is_capture_analysis_active() {
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

    pub(crate) fn platform_prepare_cached_data(&mut self, ctx: &mut compiler::CompileCtx) {
        ctx.set_persistent_cache_directory(derived_cache_directory());
    }

    pub(crate) fn platform_prepare_run(
        &mut self,
        ctx: &mut compiler::CompileCtx,
    ) -> Result<(), String> {
        self.refresh_derived_cache_nodes();
        let directory = derived_cache_directory();
        ctx.set_persistent_cache_directory(directory.clone());
        let Ok(inventory) = self
            .graph_service
            .derived_cache_configs_by_node(self.node_graph.graph(), &directory)
        else {
            // The ordinary start path reports compile errors with node
            // ownership and badges. Cache cleanup must not replace that
            // diagnostic boundary with a generic platform error.
            return Ok(());
        };
        let mut unique = std::collections::HashMap::new();
        for config in inventory.into_values().flatten() {
            unique.entry(config.cache_key).or_insert(config);
        }
        self.platform_clear_capture_caches(&unique.into_values().collect::<Vec<_>>())
            .map_err(|error| format!("Could not clear derived data cache before running: {error}"))
    }

    pub(crate) fn platform_raw_input_hook(
        &mut self,
        _ctx: &egui::Context,
        _raw_input: &mut egui::RawInput,
    ) {
    }

    pub(crate) fn platform_logic(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        while let Ok(command) = self.platform.native_menu_commands.try_recv() {
            let command = match command {
                NativeMenuCommand::About => {
                    self.about.open();
                    continue;
                }
                NativeMenuCommand::Preferences => {
                    self.preferences.open();
                    continue;
                }
                NativeMenuCommand::Run => {
                    self.run_command();
                    continue;
                }
                NativeMenuCommand::Stop => {
                    self.stop_command();
                    continue;
                }
                NativeMenuCommand::ClearDerivedCaches => {
                    self.request_clear_all_derived_caches();
                    continue;
                }
                NativeMenuCommand::ShowLogicAnalyzer => {
                    self.show_primary_panel("logic_analyzer");
                    continue;
                }
                NativeMenuCommand::ShowNodeGraph => {
                    self.show_primary_panel("node_graph");
                    continue;
                }
                NativeMenuCommand::ShowLog => {
                    self.show_auxiliary_panel("log");
                    continue;
                }
                NativeMenuCommand::ShowMemory => {
                    self.show_auxiliary_panel("memory");
                    continue;
                }
                NativeMenuCommand::ShowWatches => {
                    self.show_auxiliary_panel("watches");
                    continue;
                }
                NativeMenuCommand::ShowTriggers => {
                    self.show_auxiliary_panel("triggers");
                    continue;
                }
                NativeMenuCommand::ShowDecoder => {
                    self.show_auxiliary_panel("decoder");
                    continue;
                }
                NativeMenuCommand::ResetLaneHeights => {
                    self.reset_viewer_lane_heights();
                    continue;
                }
                NativeMenuCommand::ResetLayout => {
                    self.reset_panel_layout();
                    continue;
                }
                NativeMenuCommand::New => FileCommand::New,
                NativeMenuCommand::Load => FileCommand::Load,
                NativeMenuCommand::LoadPath(path) => FileCommand::LoadPath(path),
                NativeMenuCommand::ClearRecent => FileCommand::ClearRecent,
                NativeMenuCommand::Save => FileCommand::Save,
                NativeMenuCommand::SaveAs => FileCommand::SaveAs,
                NativeMenuCommand::SaveCaptureData => FileCommand::SaveCaptureData,
                NativeMenuCommand::Quit => FileCommand::Quit,
            };
            self.execute_file_command(command, ctx);
        }

        #[cfg(not(target_os = "macos"))]
        {
            let close_requested = ctx.input(|input| input.viewport().close_requested());
            if !self.platform.allow_close && close_requested {
                if self.has_unsaved_changes() {
                    self.platform.pending_guarded_action = Some(GuardedAction::Quit);
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                } else {
                    self.platform.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    pub(crate) fn platform_save(&mut self, storage: &mut dyn eframe::Storage) {
        self.platform.save(storage, self.node_graph.ui_prefs());
    }

    pub(crate) fn platform_before_ui(&mut self, _ui: &mut egui::Ui) {
        #[cfg(not(target_os = "macos"))]
        self.show_menu_bar(_ui);
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
                self.mark_capture_index_building();
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

    pub(crate) fn platform_before_graph(&mut self) {
        self.node_graph
            .set_derived_cache_nodes(self.platform.derived_cache_nodes.iter().copied());
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
                self.capture.clear_completed();
                self.run_message = None;
                self.error_badges.clear();
                self.apply_graph_document(graph);
                self.platform.current_file = Some(path.clone());
                self.mark_graph_saved();
                self.push_recent_file(path.clone());
                self.refresh_derived_cache_nodes();
                self.toasts.info(format!("Loaded {}", path.display()));
            }
            Err(error) => self.toasts.error(error),
        }
    }

    /// Inserts `path` at the front of the MRU list, deduping and capping at
    /// `MAX_RECENT_FILES` (Phase 5.1).
    fn push_recent_file(&mut self, path: PathBuf) {
        self.platform.push_recent_file(path);
        #[cfg(target_os = "macos")]
        notify_recent_files_changed(&self.platform.recent_files);
    }

    /// Resets to a fresh, empty graph — File → New (Phase 5.1). Assumes the
    /// unsaved-changes guard has already been resolved by the caller.
    fn do_new(&mut self) {
        if !self.can_replace_graph() {
            return;
        }
        self.clear_derived_data_presentations();
        self.capture.clear_completed();
        self.run_message = None;
        self.error_badges.clear();
        self.node_graph.new_graph();
        self.cached_preview_graph = serde_json::to_vec(self.node_graph.graph()).ok();
        self.restore_sampling_overlay_setting();
        self.restore_viewer_lane_order_setting();
        self.restore_viewer_lane_height_setting();
        self.restore_timeline_cursor_setting();
        self.restore_panel_layout_setting();
        self.platform.derived_cache_nodes.clear();
        self.platform.current_file = None;
        self.mark_graph_saved();
        self.toasts.info("New graph");
    }

    /// Requests File → New, guarding on unsaved changes the same way
    /// `request_quit` does.
    fn request_new(&mut self) {
        if self.has_unsaved_changes() {
            self.platform.pending_guarded_action = Some(GuardedAction::New);
        } else {
            self.do_new();
        }
    }

    /// Requests loading `path` (e.g. from Open Recent), guarding on unsaved
    /// changes the same way `request_quit` does.
    fn request_load_path(&mut self, path: PathBuf) {
        if self.has_unsaved_changes() {
            self.platform.pending_guarded_action = Some(GuardedAction::LoadPath(path));
        } else {
            self.load_file(path);
        }
    }

    fn choose_and_load_file(&mut self) {
        let initial_directory = self
            .platform
            .current_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_owned);
        let path = self.host_service.choose_open_file(OpenDialog {
            title: "Load graph",
            filter_label: "Graph JSON",
            extensions: &["json"],
            initial_directory: initial_directory.as_deref(),
        });
        if let Some(path) = path {
            self.load_file(path);
        }
    }

    fn save_file(&mut self) -> bool {
        let Some(path) = self.platform.current_file.clone() else {
            return self.save_file_as();
        };
        self.save_to_file(path)
    }

    fn save_file_as(&mut self) -> bool {
        let initial_directory = self
            .platform
            .current_file
            .as_ref()
            .and_then(|path| path.parent())
            .map(Path::to_owned);
        let default_file_name = self
            .platform
            .current_file
            .as_ref()
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
                self.toasts.error(error);
                return false;
            }
        };
        match self.host_service.save_graph(&path, &graph) {
            Ok(()) => {
                self.platform.current_file = Some(path.clone());
                self.mark_graph_saved();
                self.push_recent_file(path.clone());
                self.toasts.info(format!("Saved {}", path.display()));
                true
            }
            Err(error) => {
                self.toasts.error(error);
                false
            }
        }
    }

    fn choose_and_save_capture_data(&mut self) {
        let format = CaptureRawExportFormat::Portable;
        let descriptor = format.descriptor();
        let initial_directory = self
            .platform
            .current_file
            .as_ref()
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
        if let Err(error) = self.capture.start_export_current(format, path) {
            self.toasts.error(error);
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
            Ok(graph) => self.platform.saved_graph = graph,
            Err(error) => self.toasts.error(error),
        }
    }

    fn has_unsaved_changes(&mut self) -> bool {
        if self.sync_panel_layout_setting().is_err() {
            return true;
        }
        self.synchronize_payload_subscription_manifest(false);
        self.node_graph
            .snapshot_value()
            .map_or(true, |graph| graph != self.platform.saved_graph)
    }

    fn request_quit(&mut self, ctx: &egui::Context) {
        if self.has_unsaved_changes() {
            self.platform.pending_guarded_action = Some(GuardedAction::Quit);
        } else {
            self.platform.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn execute_file_command(&mut self, command: FileCommand, ctx: &egui::Context) {
        match command {
            FileCommand::New => self.request_new(),
            FileCommand::Load => self.choose_and_load_file(),
            FileCommand::LoadPath(path) => self.request_load_path(path),
            FileCommand::ClearRecent => self.platform.confirm_clear_recent = true,
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
        let continuation = match self.platform.pending_guarded_action.as_ref() {
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
        let warning_color = egui::Color32::from_rgb(240, 180, 70);
        let discard_color = egui::Color32::from_rgb(135, 55, 50);
        let style = ctx.style_of(ctx.theme());
        let modal = egui::Modal::new(egui::Id::new("unsaved-graph-changes"))
            .backdrop_color(egui::Color32::from_black_alpha(190))
            .frame(
                egui::Frame::popup(&style)
                    .fill(egui::Color32::from_rgb(47, 39, 25))
                    .stroke(egui::Stroke::new(2.0, warning_color))
                    .inner_margin(egui::Margin::symmetric(28, 24)),
            )
            .show(ctx, |ui| {
                ui.set_min_width(430.0);
                ui.label(
                    egui::RichText::new("Unsaved changes")
                        .size(26.0)
                        .strong()
                        .color(warning_color),
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
                    egui::RichText::new(
                        "Choosing Don’t Save permanently discards those changes.",
                    )
                    .color(egui::Color32::from_rgb(245, 175, 165)),
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
                                    egui::Button::new("Don’t Save").fill(discard_color),
                                )
                                .clicked()
                            {
                                choice = Some(DialogChoice::Discard);
                            }
                        },
                    );
                });
            });

        if choice.is_none()
            && modal.is_top_modal
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            choice = Some(DialogChoice::Cancel);
        }

        match choice {
            // Save can itself open a blocking Save As dialog and be
            // cancelled — leave `pending_guarded_action` set so this dialog
            // simply reopens next frame rather than silently dropping the
            // action.
            Some(DialogChoice::Save) if self.save_file() => self.complete_guarded_action(ctx),
            Some(DialogChoice::Discard) => self.complete_guarded_action(ctx),
            Some(DialogChoice::Cancel) => self.platform.pending_guarded_action = None,
            _ => {}
        }
    }

    fn complete_guarded_action(&mut self, ctx: &egui::Context) {
        let Some(action) = self.platform.pending_guarded_action.take() else {
            return;
        };
        match action {
            GuardedAction::Quit => {
                self.platform.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            GuardedAction::New => self.do_new(),
            GuardedAction::LoadPath(path) => self.load_file(path),
        }
    }

    /// Resolves the "Clear the recent files list?" confirmation triggered
    /// by either the egui or native "Clear Recent" menu item.
    fn show_clear_recent_dialog(&mut self, ctx: &egui::Context) {
        if !self.platform.confirm_clear_recent {
            return;
        }

        enum DialogChoice {
            Clear,
            Cancel,
        }

        let mut choice = None;
        egui::Window::new("Clear recent files?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Remove all entries from the recent files list?");
                ui.horizontal(|ui| {
                    if ui.button("Clear").clicked() {
                        choice = Some(DialogChoice::Clear);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(DialogChoice::Cancel);
                    }
                });
            });

        match choice {
            Some(DialogChoice::Clear) => {
                self.platform.recent_files.clear();
                #[cfg(target_os = "macos")]
                notify_recent_files_changed(&[]);
                self.platform.confirm_clear_recent = false;
            }
            Some(DialogChoice::Cancel) => self.platform.confirm_clear_recent = false,
            None => {}
        }
    }

    fn show_clear_derived_caches_dialog(&mut self, ctx: &egui::Context) {
        if !self.platform.confirm_clear_derived_caches {
            return;
        }

        let mut clear = false;
        let mut cancel = false;
        egui::Window::new("Clear all derived data caches?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label("Cached decoded data for every pipeline will be removed.");
                ui.horizontal(|ui| {
                    if ui.button("Clear All").clicked() {
                        clear = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });

        if clear {
            self.platform.confirm_clear_derived_caches = false;
            self.clear_all_derived_caches();
        } else if cancel {
            self.platform.confirm_clear_derived_caches = false;
        }
    }

    #[cfg(not(target_os = "macos"))]
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
        let mut command = if ui.input_mut(|input| input.consume_shortcut(&new_shortcut)) {
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
                        egui::Button::new("Load...")
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
                        .filter(|path| path.exists())
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
                let can_save_capture = self.capture.current_session_id().is_some()
                    && !self.capture.is_active()
                    && self.capture.export_status().is_none();
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
                        !self.is_running(),
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
        } else {
            true
        }
    }

    fn release_derived_data_handles(&mut self) {
        self.clear_derived_data_presentations();
    }

    fn refresh_derived_cache_nodes(&mut self) {
        self.platform.derived_cache_nodes = self
            .graph_service
            .derived_cache_configs_by_node(self.node_graph.graph(), &derived_cache_directory())
            .map(|inventory| {
                inventory
                    .into_keys()
                    .filter(|id| self.node_graph.graph().nodes.contains_key(id))
                    .collect()
            })
            .unwrap_or_default();
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
            .graph_service
            .derived_cache_configs_by_node(self.node_graph.graph(), &derived_cache_directory())
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
            match self.host_service.clear_cache_entry(config) {
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
            self.platform.confirm_clear_derived_caches = true;
        }
    }

    fn clear_all_derived_caches(&mut self) {
        if !self.can_clear_derived_caches() {
            return;
        }
        self.release_derived_data_handles();
        let directory = derived_cache_directory();
        match self.host_service.clear_cache(&directory) {
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
            Err(error) => self
                .toasts
                .error(format!("Failed to clear caches: {error}")),
        }
    }
}
