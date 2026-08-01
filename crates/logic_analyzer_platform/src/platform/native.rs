use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use logic_analyzer_ui::{
    APPLICATION_ID, AppServices, ApplicationSettings, ApplicationStoragePaths, CacheClearStats,
    CacheEntrySnapshot, HostCommand, HostService, OpenDialog, SaveDialog, default_input_bindings,
};
use signal_processing::PersistentStoreConfig;

use crate::services::PlatformServices;

#[cfg(target_os = "macos")]
type RecentFilesListener = Box<dyn Fn(&[PathBuf]) + Send + Sync>;

#[cfg(target_os = "macos")]
static RECENT_FILES_LISTENER: std::sync::OnceLock<RecentFilesListener> = std::sync::OnceLock::new();

#[cfg(target_os = "macos")]
pub fn set_recent_files_listener(listener: impl Fn(&[PathBuf]) + Send + Sync + 'static) {
    let _ = RECENT_FILES_LISTENER.set(Box::new(listener));
}

struct HostCommandBridge {
    sender: crossbeam_channel::Sender<HostCommand>,
    receiver: crossbeam_channel::Receiver<HostCommand>,
    repaint: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

static HOST_COMMAND_BRIDGE: std::sync::OnceLock<HostCommandBridge> = std::sync::OnceLock::new();

fn host_command_bridge() -> &'static HostCommandBridge {
    HOST_COMMAND_BRIDGE.get_or_init(|| {
        let (sender, receiver) = crossbeam_channel::unbounded();
        HostCommandBridge {
            sender,
            receiver,
            repaint: std::sync::Mutex::new(None),
        }
    })
}

#[cfg(target_os = "macos")]
pub fn dispatch_host_command(command: HostCommand) {
    queue_host_command(command);
}

fn queue_host_command(command: HostCommand) {
    let bridge = host_command_bridge();
    let _ = bridge.sender.send(command);
    if let Some(repaint) = bridge.repaint.lock().unwrap().as_ref() {
        repaint();
    }
}

pub(crate) fn standard_services() -> PlatformServices {
    let storage_paths = ApplicationStoragePaths::new(Some(derived_cache_directory()))
        .with_capture_session_directory(Some(capture_session_directory()));
    let input_bindings = load_input_bindings();
    let application_settings = load_application_settings();
    PlatformServices::with_ui_services(AppServices::with_host_storage_and_configuration(
        Box::new(NativeHostService::new()),
        storage_paths,
        input_bindings,
        application_settings,
        system_symbol_fonts(),
    ))
}

fn system_symbol_fonts() -> Vec<egui::FontData> {
    symbol_font_paths()
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .map(egui::FontData::from_owned)
        .collect()
}

#[cfg(target_os = "macos")]
fn symbol_font_paths() -> &'static [&'static str] {
    &["/System/Library/Fonts/Apple Symbols.ttf"]
}

#[cfg(target_os = "windows")]
fn symbol_font_paths() -> &'static [&'static str] {
    &[r"C:\Windows\Fonts\seguisym.ttf"]
}

#[cfg(target_os = "linux")]
fn symbol_font_paths() -> &'static [&'static str] {
    &[
        "/usr/share/fonts/truetype/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-symbols2-fonts/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-symbols-fonts/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/google-noto-sans-math-fonts/NotoSansMath-Regular.ttf",
        "/usr/local/share/NotoSansSymbols2-Regular.ttf",
        "/usr/local/share/NotoSansSymbols-Regular.ttf",
        "/usr/local/share/NotoSansMath-Regular.ttf",
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn symbol_font_paths() -> &'static [&'static str] {
    &[]
}

fn load_application_settings() -> ApplicationSettings {
    let Some(path) = configuration_file("application.json") else {
        return ApplicationSettings::default();
    };
    load_application_settings_path(&path)
}

fn load_application_settings_path(path: &Path) -> ApplicationSettings {
    match std::fs::read_to_string(path) {
        Ok(json) => ApplicationSettings::from_json(&json).unwrap_or_else(|error| {
            panic!(
                "invalid application configuration in {}: {error}",
                path.display()
            )
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => ApplicationSettings::default(),
        Err(error) => panic!(
            "cannot read application configuration from {}: {error}",
            path.display()
        ),
    }
}

fn load_input_bindings() -> input_bindings::InputBindings {
    let Some(path) = configuration_file("input_bindings.json") else {
        return default_input_bindings();
    };
    load_input_bindings_path(&path)
}

fn load_input_bindings_path(path: &Path) -> input_bindings::InputBindings {
    match std::fs::read_to_string(path) {
        Ok(json) => input_bindings::InputBindings::from_json(&json).unwrap_or_else(|error| {
            panic!("invalid input bindings in {}: {error}", path.display())
        }),
        Err(error) if error.kind() == ErrorKind::NotFound => default_input_bindings(),
        Err(error) => panic!(
            "cannot read input bindings from {}: {error}",
            path.display()
        ),
    }
}

fn configuration_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|directory| directory.join(APPLICATION_ID).join(name))
}

fn derived_cache_directory() -> PathBuf {
    application_cache_directory().join("derived")
}

fn capture_session_directory() -> PathBuf {
    application_cache_directory().join("captures")
}

fn application_cache_directory() -> PathBuf {
    std::cfg_select! {
        target_os = "macos" => {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| application_directory(home.join("Library").join("Caches")))
                .unwrap_or_else(|| application_directory(std::env::temp_dir()))
        }
        target_os = "windows" => {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(application_directory)
                .unwrap_or_else(|| application_directory(std::env::temp_dir()))
        }
        _ => {
            std::env::var_os("XDG_CACHE_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .map(|home| home.join(".cache"))
                })
                .map(application_directory)
                .unwrap_or_else(|| application_directory(std::env::temp_dir()))
        }
    }
}

fn application_directory(parent: PathBuf) -> PathBuf {
    parent.join(APPLICATION_ID)
}

struct NativeHostService {
    commands: crossbeam_channel::Receiver<HostCommand>,
}

impl NativeHostService {
    fn new() -> Self {
        Self {
            commands: host_command_bridge().receiver.clone(),
        }
    }
}

impl HostService for NativeHostService {
    fn set_command_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        *host_command_bridge().repaint.lock().unwrap() = Some(repaint);
    }

    fn take_commands(&mut self) -> Vec<HostCommand> {
        self.commands.try_iter().collect()
    }

    fn publish_recent_files(&self, paths: &[PathBuf]) {
        #[cfg(target_os = "macos")]
        if let Some(listener) = RECENT_FILES_LISTENER.get() {
            listener(paths);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = paths;
    }

    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_file()
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .set_file_name(request.default_file_name)
            .add_filter(request.filter_label, request.extensions);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.save_file()
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        rfd::FileDialog::new().pick_folder()
    }

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        serde_json::from_str(&json)
            .map_err(|error| format!("could not parse {}: {error}", path.display()))
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        let json = serde_json::to_string_pretty(graph)
            .map_err(|error| format!("could not serialize graph: {error}"))?;
        std::fs::write(path, json)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    fn clear_cache_entry(
        &mut self,
        config: &PersistentStoreConfig,
    ) -> Result<CacheClearStats, String> {
        signal_processing::clear_cache_entry(config)
            .map(|stats| CacheClearStats {
                removed_entries: stats.removed_entries,
                removed_bytes: stats.removed_bytes,
            })
            .map_err(|error| error.to_string())
    }

    fn clear_cache(&mut self, directory: &Path) -> Result<CacheClearStats, String> {
        signal_processing::clear_cache(directory)
            .map(|stats| CacheClearStats {
                removed_entries: stats.removed_entries,
                removed_bytes: stats.removed_bytes,
            })
            .map_err(|error| error.to_string())
    }

    fn inspect_cache_entry(
        &self,
        config: &PersistentStoreConfig,
    ) -> Result<Option<CacheEntrySnapshot>, String> {
        signal_processing::derived_word_store::inspect_cache_entry(config)
            .map(|entry| {
                entry.map(|entry| CacheEntrySnapshot {
                    total_bytes: entry.total_bytes,
                    data_bytes: entry.data_bytes,
                    index_bytes: entry.index_bytes,
                    item_count: entry.word_count,
                    index_item_count: entry.block_count as u64,
                    first_timestamp_ns: entry.first_timestamp_ns,
                    last_timestamp_ns: entry.last_timestamp_ns,
                })
            })
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod native_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use logic_analyzer_ui::{HostCommand, HostService};
    use logic_analyzer_viewer::ColorProfile;

    use super::{
        NativeHostService, application_directory, load_application_settings_path,
        load_input_bindings_path, queue_host_command,
    };

    #[test]
    fn native_shell_commands_wake_and_reach_the_ui_service_port() {
        let repaint_count = Arc::new(AtomicUsize::new(0));
        let callback_count = Arc::clone(&repaint_count);
        let mut host = NativeHostService::new();
        host.set_command_repaint(Box::new(move || {
            callback_count.fetch_add(1, Ordering::Relaxed);
        }));

        queue_host_command(HostCommand::Run);

        assert_eq!(repaint_count.load(Ordering::Relaxed), 1);
        assert_eq!(host.take_commands(), vec![HostCommand::Run]);
    }

    #[test]
    fn native_cache_directories_use_the_application_identifier() {
        let parent = tempfile::tempdir().unwrap();

        assert_eq!(
            application_directory(parent.path().to_owned()),
            parent.path().join("logic-conduit")
        );
    }

    #[test]
    fn native_configuration_files_override_embedded_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let application = directory.path().join("application.json");
        let input_bindings = directory.path().join("input_bindings.json");
        std::fs::write(
            &application,
            r#"{
                "logic_analyzer_viewer": { "color_profile": "classic" },
                "live_capture": { "max_recent_sessions": 7, "max_storage_gib": 12 }
            }"#,
        )
        .unwrap();
        std::fs::write(
            &input_bindings,
            r#"{"bindings":[
                {"context":"custom","action":"only","label":"Only","input":"key","key":"f12"}
            ]}"#,
        )
        .unwrap();

        let settings = load_application_settings_path(&application);
        let bindings = load_input_bindings_path(&input_bindings);

        assert_eq!(settings.viewer_color_profile(), ColorProfile::Classic);
        assert_eq!(settings.max_recent_capture_sessions(), 7);
        assert_eq!(settings.max_capture_storage_gib(), 12);
        assert!(bindings.shortcut(&["custom"], "only").is_some());
        assert!(bindings.shortcut(&["global"], "save").is_none());
    }

    #[test]
    fn missing_native_configuration_files_use_embedded_defaults() {
        let directory = tempfile::tempdir().unwrap();

        let settings = load_application_settings_path(&directory.path().join("missing.json"));
        let bindings = load_input_bindings_path(&directory.path().join("missing.json"));

        assert_eq!(settings.viewer_color_profile(), ColorProfile::DsView);
        assert!(bindings.shortcut(&["global"], "save").is_some());
    }
}
