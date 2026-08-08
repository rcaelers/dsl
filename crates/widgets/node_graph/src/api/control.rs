use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Rect, Ui};
use thiserror::Error;

/// Failure returned by an injected file-dialog service.
#[derive(Debug, Error)]
pub enum FileDialogError {
    /// A dropped file did not include a path or importable contents.
    #[error("the host did not provide contents for '{name}'")]
    ContentsUnavailable {
        /// Display name supplied by the host.
        name: String,
    },
    /// The injected host service reported a typed failure.
    #[error("{source}")]
    Host {
        /// Concrete host-adapter cause retained until widget presentation.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl FileDialogError {
    /// Retains an injected host service failure without formatting it.
    pub fn host(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Host {
            source: Box::new(error),
        }
    }
}

/// File-type filter offered by a host file picker.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileDialogFilter {
    /// User-visible filter name.
    pub name: String,
    /// Accepted filename extensions without the leading dot.
    pub extensions: Vec<String>,
}

/// Request sent from an inline file control to the host file-picker service.
pub struct FileDialogRequest<'a> {
    /// Caller-chosen identifier used to retrieve asynchronous completion.
    pub request_id: u64,
    /// User-facing dialog title.
    pub title: &'a str,
    /// File types selectable in the dialog.
    pub filters: &'a [FileDialogFilter],
    /// Whether the dialog should select a destination rather than an existing file.
    pub save: bool,
}

/// Host-neutral progress for an asynchronous file selection or import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDialogProgress {
    /// Bytes selected, read, or imported so far.
    pub completed_bytes: u64,
    /// Total bytes when the host can determine the size.
    pub total_bytes: Option<u64>,
}

/// Host-neutral file data delivered by drag-and-drop.
pub struct DroppedFile {
    /// Host-provided display name.
    pub name: String,
    /// Native path when the host exposes one.
    pub path: Option<PathBuf>,
    /// File content when the host provides dropped bytes instead of a path.
    pub bytes: Option<Arc<[u8]>>,
}

/// Host capability used by inline controls to choose and import files.
pub trait FileDialogService: Send {
    /// Returns whether the host supports opening the requested picker mode.
    ///
    /// # Parameters
    /// - `save`: Whether the control needs a save rather than open picker.
    fn available(&self, save: bool) -> bool;

    /// Starts a file selection and optionally returns an immediate result.
    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String>;

    /// Returns a completed asynchronous selection for one control.
    fn take_picked(&mut self, _request_id: u64) -> Option<Result<String, FileDialogError>> {
        None
    }

    /// Returns current asynchronous selection/import progress.
    fn progress(&self, _request_id: u64) -> Option<FileDialogProgress> {
        None
    }

    /// Cancels a pending asynchronous selection/import when supported.
    fn cancel(&mut self, _request_id: u64) -> bool {
        false
    }

    /// Imports bytes supplied by the host's drag-and-drop mechanism.
    fn import_dropped(&mut self, file: DroppedFile) -> Result<String, FileDialogError> {
        let name = file.name;
        file.path
            .map(|path| path.display().to_string())
            .ok_or(FileDialogError::ContentsUnavailable { name })
    }

    /// Installs the wake-up callback used by asynchronous dialog implementations.
    fn set_repaint(&mut self, _repaint: Box<dyn Fn() + Send + Sync>) {}
}

/// Capability-limited host services available while rendering an inline control.
pub struct InlineControlContext<'a> {
    file_dialog: &'a mut dyn FileDialogService,
}

impl<'a> InlineControlContext<'a> {
    pub(crate) fn new(file_dialog: &'a mut dyn FileDialogService) -> Self {
        Self { file_dialog }
    }

    /// Returns whether the host supports the requested picker mode.
    pub fn file_dialog_available(&self, save: bool) -> bool {
        self.file_dialog.available(save)
    }

    /// Starts a file picker and optionally returns an immediate selection.
    pub fn pick_file(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        self.file_dialog.pick(request)
    }

    /// Takes picked file, leaving its default state.
    pub fn take_picked_file(&mut self, request_id: u64) -> Option<Result<String, FileDialogError>> {
        self.file_dialog.take_picked(request_id)
    }

    /// Returns progress for an asynchronous file selection or import.
    pub fn picked_file_progress(&self, request_id: u64) -> Option<FileDialogProgress> {
        self.file_dialog.progress(request_id)
    }

    /// Cancels an asynchronous file selection when supported.
    pub fn cancel_picked_file(&mut self, request_id: u64) -> bool {
        self.file_dialog.cancel(request_id)
    }

    /// Imports a host-supplied dropped file into a control value.
    pub fn import_dropped_file(&mut self, file: DroppedFile) -> Result<String, FileDialogError> {
        self.file_dialog.import_dropped(file)
    }
}

pub(crate) struct UnavailableFileDialogService;

impl FileDialogService for UnavailableFileDialogService {
    fn available(&self, _save: bool) -> bool {
        false
    }

    fn pick(&mut self, _request: FileDialogRequest<'_>) -> Option<String> {
        None
    }
}

/// Editable inline UI state bound to a node-state field.
pub trait InlineControl: Send + Sync + fmt::Debug {
    /// Draws the state-bound inline editor and returns whether it changed the value.
    ///
    /// # Parameters
    /// - `ui`: Parent UI receiving input events.
    /// - `label`: User-facing socket label.
    /// - `rect`: Screen-space area allocated to the control.
    /// - `zoom`: Current graph canvas zoom factor.
    /// - `clip_rect`: Screen-space clipping rectangle for control painting.
    /// - `context`: Host capabilities available while rendering the control.
    fn draw_widget(
        &mut self,
        ui: &mut Ui,
        label: &str,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        context: &mut InlineControlContext<'_>,
    ) -> bool;
}
