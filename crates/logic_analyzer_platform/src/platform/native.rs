use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::MmapOptions;

use logic_analyzer_graph_compiler::{
    SourcePreparationExecutor, SourcePreparationResult, SourcePreparationTask,
    SourcePreparationTaskUpdate, SourcePreparationWork,
};
use logic_analyzer_ui::{
    APPLICATION_ID, AppServices, ApplicationSettings, ApplicationStoragePaths, CacheClearStats,
    CacheEntrySnapshot, DecodedBlockCacheSnapshot, HostCommand, HostService, OpenDialog,
    SaveDialog, default_input_bindings,
};
use node_graph::{FileDialogRequest, FileDialogService};
use signal_processing::{
    ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactRepository, ByteRange, ByteRegion,
    ImmutableByteRegion, PersistentStoreConfig, ReadArtifact, RepositoryCapabilities,
    RepositoryError, SourceIdentity, WriteArtifact,
};

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
    let ui_services = AppServices::with_host_storage_and_configuration(
        Box::new(NativeHostService::new()),
        storage_paths,
        input_bindings,
        application_settings,
        system_symbol_fonts(),
    )
    .with_node_file_dialog(Box::new(NativeNodeFileDialogService))
    .with_source_preparation_executor(Box::new(NativeSourcePreparationExecutor::new()));
    PlatformServices::with_ui_services(
        ui_services,
        Arc::new(NativeArtifactRepository::new(
            derived_cache_directory().join("artifacts"),
        )),
    )
}

struct NativeSourcePreparationExecutor {
    sender: crossbeam_channel::Sender<QueuedSourcePreparation>,
}

impl NativeSourcePreparationExecutor {
    fn new() -> Self {
        const WORKERS: usize = 1;
        let (sender, receiver) = crossbeam_channel::bounded(WORKERS * 2);
        for index in 0..WORKERS {
            let receiver = receiver.clone();
            std::thread::Builder::new()
                .name(format!("source-preparation-{index}"))
                .spawn(move || run_source_preparation_worker(receiver))
                .expect("failed to start source preparation worker");
        }
        Self { sender }
    }
}

impl SourcePreparationExecutor for NativeSourcePreparationExecutor {
    fn submit(
        &self,
        work: SourcePreparationWork,
    ) -> Result<Box<dyn SourcePreparationTask>, String> {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        self.sender
            .try_send(QueuedSourcePreparation {
                work,
                result_sender: sender,
            })
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => {
                    String::from("source-preparation worker queue is full")
                }
                crossbeam_channel::TrySendError::Disconnected(_) => {
                    String::from("source-preparation worker stopped")
                }
            })?;
        Ok(Box::new(NativeSourcePreparationTask { receiver }))
    }
}

struct QueuedSourcePreparation {
    work: SourcePreparationWork,
    result_sender: crossbeam_channel::Sender<SourcePreparationResult>,
}

fn run_source_preparation_worker(receiver: crossbeam_channel::Receiver<QueuedSourcePreparation>) {
    while let Ok(QueuedSourcePreparation {
        work,
        result_sender,
    }) = receiver.recv()
    {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .unwrap_or_else(|_| Err("source-preparation worker panicked".into()));
        let _ = result_sender.send(result);
    }
}

struct NativeSourcePreparationTask {
    receiver: crossbeam_channel::Receiver<SourcePreparationResult>,
}

impl SourcePreparationTask for NativeSourcePreparationTask {
    fn poll(&mut self) -> SourcePreparationTaskUpdate {
        match self.receiver.try_recv() {
            Ok(result) => SourcePreparationTaskUpdate::Complete(result),
            Err(crossbeam_channel::TryRecvError::Empty) => SourcePreparationTaskUpdate::Pending,
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                SourcePreparationTaskUpdate::Disconnected
            }
        }
    }
}

static NEXT_ARTIFACT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(1);

/// Native persistent artifact repository selected by the platform bundle.
///
/// The repository stores an artifact under an opaque identity-derived name,
/// publishes it with a same-directory rename, and exposes immutable reads as
/// mmap-backed byte regions. Storage algorithms see only the owner contracts.
struct NativeArtifactRepository {
    root: PathBuf,
}

impl NativeArtifactRepository {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn namespace_directory(&self, namespace: &ArtifactNamespace) -> PathBuf {
        self.root.join(hex_encode(namespace.as_str().as_bytes()))
    }

    fn artifact_path(&self, key: &ArtifactKey) -> PathBuf {
        self.namespace_directory(key.namespace())
            .join(hex_encode(key.identity().as_bytes()))
    }

    fn temporary_path(&self, key: &ArtifactKey) -> PathBuf {
        let id = NEXT_ARTIFACT_TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let artifact_name = hex_encode(key.identity().as_bytes());
        self.namespace_directory(key.namespace())
            .join(format!(".{artifact_name}.{id}.pending"))
    }
}

impl ArtifactRepository for NativeArtifactRepository {
    fn capabilities(&self) -> RepositoryCapabilities {
        RepositoryCapabilities {
            durable: true,
            atomic_publication: true,
            immutable_regions: true,
        }
    }

    fn open(&self, key: &ArtifactKey) -> Result<Option<Box<dyn ReadArtifact>>, RepositoryError> {
        let path = self.artifact_path(key);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(repository_io(error)),
        };
        let length = file.metadata().map_err(repository_io)?.len();
        let backing = if length == 0 {
            NativeArtifactRegion::Empty
        } else {
            // SAFETY: the artifact is immutable after same-directory atomic
            // publication. This read handle owns the mmap for its lifetime.
            let map = unsafe { MmapOptions::new().map(&file) }.map_err(repository_io)?;
            NativeArtifactRegion::Mapped(map)
        };
        Ok(Some(Box::new(NativeReadArtifact {
            key: key.clone(),
            backing: Arc::new(backing),
            length,
        })))
    }

    fn begin_write(&self, key: ArtifactKey) -> Result<Box<dyn WriteArtifact>, RepositoryError> {
        let directory = self.namespace_directory(key.namespace());
        std::fs::create_dir_all(&directory).map_err(repository_io)?;
        let temporary_path = self.temporary_path(&key);
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&temporary_path)
            .map_err(repository_io)?;
        let final_path = self.artifact_path(&key);
        Ok(Box::new(NativeWriteArtifact {
            key,
            file: Some(file),
            temporary_path,
            final_path,
            published: false,
        }))
    }

    fn remove(&self, key: &ArtifactKey) -> Result<(), RepositoryError> {
        match std::fs::remove_file(self.artifact_path(key)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(repository_io(error)),
        }
    }

    fn entries(
        &self,
        namespace: &ArtifactNamespace,
    ) -> Result<Vec<ArtifactMetadata>, RepositoryError> {
        let directory = self.namespace_directory(namespace);
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(repository_io(error)),
        };
        let mut artifacts = Vec::new();
        for entry in entries {
            let entry = entry.map_err(repository_io)?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(identity) = parse_identity(&name) else {
                continue;
            };
            let metadata = entry.metadata().map_err(repository_io)?;
            if !metadata.is_file() {
                continue;
            }
            artifacts.push(ArtifactMetadata {
                key: ArtifactKey::new(namespace.clone(), identity),
                length: metadata.len(),
            });
        }
        artifacts.sort_by(|left, right| left.key.cmp(&right.key));
        Ok(artifacts)
    }
}

enum NativeArtifactRegion {
    // Mmaps are only constructed from a finalized immutable artifact.
    Mapped(memmap2::Mmap),
    Empty,
}

impl ImmutableByteRegion for NativeArtifactRegion {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Mapped(map) => map,
            Self::Empty => &[],
        }
    }
}

struct NativeReadArtifact {
    key: ArtifactKey,
    backing: Arc<NativeArtifactRegion>,
    length: u64,
}

impl ReadArtifact for NativeReadArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn len(&self) -> Result<u64, RepositoryError> {
        Ok(self.length)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, RepositoryError> {
        let start = usize::try_from(offset).map_err(|_| RepositoryError::OutOfBounds {
            offset,
            end: u64::MAX,
            artifact_length: self.length,
        })?;
        if start >= self.backing.bytes().len() {
            return Ok(0);
        }
        let source = &self.backing.bytes()[start..];
        let count = source.len().min(destination.len());
        destination[..count].copy_from_slice(&source[..count]);
        Ok(count)
    }

    fn region(&self, range: ByteRange) -> Result<Option<ByteRegion>, RepositoryError> {
        if range.end() > self.length {
            return Err(RepositoryError::OutOfBounds {
                offset: range.offset,
                end: range.end(),
                artifact_length: self.length,
            });
        }
        let backing: Arc<dyn ImmutableByteRegion> = self.backing.clone();
        ByteRegion::new(backing, range)
            .map(Some)
            .map_err(RepositoryError::from)
    }
}

struct NativeWriteArtifact {
    key: ArtifactKey,
    file: Option<File>,
    temporary_path: PathBuf,
    final_path: PathBuf,
    published: bool,
}

impl NativeWriteArtifact {
    fn file_mut(&mut self) -> Result<&mut File, RepositoryError> {
        self.file
            .as_mut()
            .ok_or_else(|| RepositoryError::Io("artifact write was already published".into()))
    }
}

impl WriteArtifact for NativeWriteArtifact {
    fn key(&self) -> &ArtifactKey {
        &self.key
    }

    fn write_at(&mut self, offset: u64, source: &[u8]) -> Result<(), RepositoryError> {
        self.file_mut()?
            .seek(SeekFrom::Start(offset))
            .map_err(repository_io)?;
        self.file_mut()?.write_all(source).map_err(repository_io)
    }

    fn truncate(&mut self, len: u64) -> Result<(), RepositoryError> {
        self.file_mut()?.set_len(len).map_err(repository_io)
    }

    fn flush(&mut self) -> Result<(), RepositoryError> {
        self.file_mut()?.sync_all().map_err(repository_io)
    }

    fn publish(mut self: Box<Self>) -> Result<(), RepositoryError> {
        let file = self
            .file
            .take()
            .ok_or_else(|| RepositoryError::Io("artifact write was already published".into()))?;
        file.sync_all().map_err(repository_io)?;
        drop(file);
        std::fs::rename(&self.temporary_path, &self.final_path).map_err(repository_io)?;
        self.published = true;
        Ok(())
    }
}

impl Drop for NativeWriteArtifact {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.temporary_path);
        }
    }
}

fn repository_io(error: std::io::Error) -> RepositoryError {
    RepositoryError::Io(error.to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn parse_identity(value: &str) -> Option<SourceIdentity> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    let (pairs, []) = value.as_bytes().as_chunks::<2>() else {
        return None;
    };
    for (index, pair) in pairs.iter().enumerate() {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        bytes[index] = (high << 4) | low;
    }
    Some(SourceIdentity::from_bytes(bytes))
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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

struct NativeNodeFileDialogService;

impl FileDialogService for NativeNodeFileDialogService {
    fn available(&self) -> bool {
        true
    }

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        let mut dialog = rfd::FileDialog::new();
        if !request.title.is_empty() {
            dialog = dialog.set_title(request.title);
        }
        for filter in request.filters {
            let extensions = filter
                .extensions
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            dialog = dialog.add_filter(&filter.name, &extensions);
        }
        let selected = if request.save {
            dialog.save_file()
        } else {
            dialog.pick_file()
        };
        selected.map(|path| path.display().to_string())
    }
}

impl NativeHostService {
    fn new() -> Self {
        Self {
            commands: host_command_bridge().receiver.clone(),
        }
    }
}

impl HostService for NativeHostService {
    fn decoded_block_cache_snapshot(&self) -> Option<DecodedBlockCacheSnapshot> {
        let stats = signal_processing::decoded_block_cache_stats();
        Some(DecodedBlockCacheSnapshot {
            entries: stats.entries,
            memory_bytes: stats.memory_bytes,
            budget_bytes: stats.budget_bytes,
            hits: stats.hits,
            misses: stats.misses,
        })
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

    use logic_analyzer_graph_compiler::{
        PreparedCaptureData, SourcePreparationExecutor, SourcePreparationTaskUpdate,
    };
    use logic_analyzer_ui::{HostCommand, HostService};
    use signal_processing::{
        ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange, SourceIdentity,
    };

    use super::{
        NativeArtifactRepository, NativeHostService, NativeSourcePreparationExecutor,
        application_directory, load_application_settings_path, load_input_bindings_path,
        queue_host_command,
    };

    #[test]
    fn native_source_preparation_executor_completes_work_off_the_caller() {
        let executor = NativeSourcePreparationExecutor::new();
        let mut task = executor
            .submit(Box::new(|| {
                Ok(PreparedCaptureData::Channels(vec![(4, "Data".into())]))
            }))
            .unwrap();

        for _ in 0..10_000 {
            match task.poll() {
                SourcePreparationTaskUpdate::Pending => std::thread::yield_now(),
                SourcePreparationTaskUpdate::Complete(Ok(PreparedCaptureData::Channels(
                    channels,
                ))) => {
                    assert_eq!(channels, vec![(4, "Data".into())]);
                    return;
                }
                SourcePreparationTaskUpdate::Complete(Ok(_)) => {
                    panic!("source preparation returned the wrong data kind");
                }
                SourcePreparationTaskUpdate::Complete(Err(error)) => {
                    panic!("source preparation failed: {error}");
                }
                SourcePreparationTaskUpdate::Disconnected => {
                    panic!("source preparation worker disconnected");
                }
            }
        }
        panic!("source preparation worker did not complete");
    }

    #[test]
    fn native_artifact_repository_publishes_mmap_backed_artifacts_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let repository = NativeArtifactRepository::new(directory.path().join("artifacts"));
        let namespace = ArtifactNamespace::new("derived payload").unwrap();
        let key = ArtifactKey::new(namespace.clone(), SourceIdentity::from_bytes([0x5a; 32]));

        let mut writer = repository.begin_write(key.clone()).unwrap();
        writer.write_at(2, b"cde").unwrap();
        writer.write_at(0, b"ab").unwrap();
        writer.publish().unwrap();

        let mut reader = repository.open(&key).unwrap().unwrap();
        let mut bytes = [0_u8; 5];
        reader.read_at(0, &mut bytes).unwrap();
        assert_eq!(&bytes, b"abcde");
        assert_eq!(
            reader
                .region(ByteRange::new(1, 3).unwrap())
                .unwrap()
                .unwrap()
                .bytes(),
            b"bcd"
        );
        assert_eq!(repository.entries(&namespace).unwrap().len(), 1);
        assert!(repository.capabilities().durable);
        assert!(repository.capabilities().atomic_publication);
        assert!(repository.capabilities().immutable_regions);

        let mut unpublished = repository.begin_write(key.clone()).unwrap();
        unpublished.write_at(0, b"incomplete").unwrap();
        drop(unpublished);

        let mut reader = repository.open(&key).unwrap().unwrap();
        let mut preserved = [0_u8; 5];
        reader.read_at(0, &mut preserved).unwrap();
        assert_eq!(&preserved, b"abcde");
    }

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

        assert_eq!(settings.max_recent_capture_sessions(), 10);
        assert_eq!(settings.max_capture_storage_gib(), 20);
        assert!(bindings.shortcut(&["global"], "save").is_some());
    }
}
