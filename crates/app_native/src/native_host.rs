//! Native adaptation of low-level host mechanisms to UI-owned ports.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use logic_analyzer_ui::{
    APPLICATION_ID, ApplicationSettings, GraphDocumentError, HostCommand, HostService,
    HostUiCapabilities, ModifierKeyLabels, OpenDialog, SaveDialog, default_input_bindings,
};
use node_graph::{FileDialogRequest, FileDialogService};
use platform::{FileDialogFilter, FileOpenDialog, FileSaveDialog, NativeDocumentHost};

#[cfg(target_os = "macos")]
type RecentFilesListener = Box<dyn Fn(&[PathBuf]) + Send + Sync>;

#[cfg(target_os = "macos")]
static RECENT_FILES_LISTENER: OnceLock<RecentFilesListener> = OnceLock::new();

struct HostCommandBridge {
    sender: crossbeam_channel::Sender<HostCommand>,
    receiver: crossbeam_channel::Receiver<HostCommand>,
    repaint: std::sync::Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

static HOST_COMMAND_BRIDGE: OnceLock<HostCommandBridge> = OnceLock::new();

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
pub(crate) fn set_recent_files_listener(listener: impl Fn(&[PathBuf]) + Send + Sync + 'static) {
    let _ = RECENT_FILES_LISTENER.set(Box::new(listener));
}

#[cfg(target_os = "macos")]
pub(crate) fn dispatch_host_command(command: HostCommand) {
    let bridge = host_command_bridge();
    let _ = bridge.sender.send(command);
    if let Some(repaint) = bridge.repaint.lock().unwrap().as_ref() {
        repaint();
    }
}

pub(crate) struct NativeHostService {
    commands: crossbeam_channel::Receiver<HostCommand>,
    documents: NativeDocumentHost,
}

impl NativeHostService {
    pub(crate) fn new() -> Self {
        Self {
            commands: host_command_bridge().receiver.clone(),
            documents: NativeDocumentHost::new(),
        }
    }
}

impl HostService for NativeHostService {
    fn ui_capabilities(&self) -> HostUiCapabilities {
        #[cfg(target_os = "macos")]
        {
            HostUiCapabilities {
                direct_document_access: true,
                system_menu_bar: true,
                viewport_close_guard: false,
                modifier_key_labels: ModifierKeyLabels {
                    alternate: "Option",
                    command: "Command",
                },
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            HostUiCapabilities {
                direct_document_access: true,
                system_menu_bar: false,
                viewport_close_guard: true,
                modifier_key_labels: ModifierKeyLabels::default(),
            }
        }
    }

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

    fn document_exists(&self, path: &Path) -> bool {
        self.documents.exists(path)
    }

    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        self.documents.choose_open_file(FileOpenDialog {
            title: request.title.to_owned(),
            filters: vec![FileDialogFilter {
                name: request.filter_label.to_owned(),
                extensions: request
                    .extensions
                    .iter()
                    .map(|extension| (*extension).to_owned())
                    .collect(),
            }],
            initial_directory: request.initial_directory.map(Path::to_owned),
        })
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        self.documents.choose_save_file(FileSaveDialog {
            title: request.title.to_owned(),
            default_file_name: request.default_file_name.to_owned(),
            filters: vec![FileDialogFilter {
                name: request.filter_label.to_owned(),
                extensions: request
                    .extensions
                    .iter()
                    .map(|extension| (*extension).to_owned())
                    .collect(),
            }],
            initial_directory: request.initial_directory.map(Path::to_owned),
        })
    }

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, GraphDocumentError> {
        let json = self
            .documents
            .read(path)
            .map_err(|error| GraphDocumentError::read(path, error))?;
        serde_json::from_slice(&json).map_err(|source| GraphDocumentError::Decode {
            path: path.to_owned(),
            source,
        })
    }

    fn save_graph(
        &mut self,
        path: &Path,
        graph: &serde_json::Value,
    ) -> Result<(), GraphDocumentError> {
        let json = serde_json::to_vec_pretty(graph)
            .map_err(|source| GraphDocumentError::Encode { source })?;
        self.documents
            .write(path, &json)
            .map_err(|error| GraphDocumentError::write(path, error))
    }
}

pub(crate) struct NativeNodeFileDialogService {
    documents: NativeDocumentHost,
}

impl NativeNodeFileDialogService {
    pub(crate) fn new() -> Self {
        Self {
            documents: NativeDocumentHost::new(),
        }
    }
}

impl FileDialogService for NativeNodeFileDialogService {
    fn available(&self, _save: bool) -> bool {
        true
    }

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        let filters = request
            .filters
            .iter()
            .map(|filter| FileDialogFilter {
                name: filter.name.clone(),
                extensions: filter.extensions.clone(),
            })
            .collect();
        let selected = if request.save {
            self.documents.choose_save_file(FileSaveDialog {
                title: request.title.to_owned(),
                default_file_name: String::new(),
                filters,
                initial_directory: None,
            })
        } else {
            self.documents.choose_open_file(FileOpenDialog {
                title: request.title.to_owned(),
                filters,
                initial_directory: None,
            })
        };
        selected.map(|path| path.display().to_string())
    }
}

pub(crate) fn application_settings() -> ApplicationSettings {
    let documents = NativeDocumentHost::new();
    let path = documents.configuration_file(APPLICATION_ID, "application.json");
    load_application_settings(&documents, &path)
}

pub(crate) fn input_bindings() -> input_bindings::InputBindings {
    let documents = NativeDocumentHost::new();
    let path = documents.configuration_file(APPLICATION_ID, "input_bindings.json");
    load_input_bindings(&documents, &path)
}

pub(crate) fn system_symbol_fonts() -> Vec<egui::FontData> {
    let documents = NativeDocumentHost::new();
    symbol_font_paths()
        .iter()
        .filter_map(|path| documents.read_optional(Path::new(path)).ok().flatten())
        .map(egui::FontData::from_owned)
        .collect()
}

fn load_application_settings(documents: &NativeDocumentHost, path: &Path) -> ApplicationSettings {
    match documents.read_optional(path) {
        Ok(Some(json)) => {
            ApplicationSettings::from_json(std::str::from_utf8(&json).unwrap_or_else(|error| {
                panic!(
                    "invalid application configuration encoding in {}: {error}",
                    path.display()
                )
            }))
            .unwrap_or_else(|error| {
                panic!(
                    "invalid application configuration in {}: {error}",
                    path.display()
                )
            })
        }
        Ok(None) => ApplicationSettings::default(),
        Err(error) => panic!("cannot read application configuration: {error}"),
    }
}

fn load_input_bindings(
    documents: &NativeDocumentHost,
    path: &Path,
) -> input_bindings::InputBindings {
    match documents.read_optional(path) {
        Ok(Some(json)) => input_bindings::InputBindings::from_json(
            std::str::from_utf8(&json).unwrap_or_else(|error| {
                panic!(
                    "invalid input bindings encoding in {}: {error}",
                    path.display()
                )
            }),
        )
        .unwrap_or_else(|error| panic!("invalid input bindings in {}: {error}", path.display())),
        Ok(None) => default_input_bindings(),
        Err(error) => panic!("cannot read input bindings: {error}"),
    }
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
        "/usr/share/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/noto/NotoSansMath-Regular.ttf",
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
