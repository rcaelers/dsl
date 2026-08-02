use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Rect, Ui};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileDialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

pub struct FileDialogRequest<'a> {
    pub request_id: u64,
    pub title: &'a str,
    pub filters: &'a [FileDialogFilter],
    pub save: bool,
}

/// Host-neutral file data delivered by drag-and-drop.
pub struct DroppedFile {
    pub name: String,
    pub path: Option<PathBuf>,
    pub bytes: Option<Arc<[u8]>>,
}

pub trait FileDialogService: Send {
    fn available(&self, save: bool) -> bool;

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String>;

    /// Returns a completed asynchronous selection for one control.
    fn take_picked(&mut self, _request_id: u64) -> Option<Result<String, String>> {
        None
    }

    /// Imports bytes supplied by the host's drag-and-drop mechanism.
    fn import_dropped(&mut self, file: DroppedFile) -> Result<String, String> {
        file.path
            .map(|path| path.display().to_string())
            .ok_or_else(|| "this host cannot import dropped file bytes".to_owned())
    }

    /// Installs the wake-up callback used by asynchronous dialog implementations.
    fn set_repaint(&mut self, _repaint: Box<dyn Fn() + Send + Sync>) {}
}

pub struct InlineControlContext<'a> {
    file_dialog: &'a mut dyn FileDialogService,
}

impl<'a> InlineControlContext<'a> {
    pub(crate) fn new(file_dialog: &'a mut dyn FileDialogService) -> Self {
        Self { file_dialog }
    }

    pub fn file_dialog_available(&self, save: bool) -> bool {
        self.file_dialog.available(save)
    }

    pub fn pick_file(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        self.file_dialog.pick(request)
    }

    pub fn take_picked_file(&mut self, request_id: u64) -> Option<Result<String, String>> {
        self.file_dialog.take_picked(request_id)
    }

    pub fn import_dropped_file(&mut self, file: DroppedFile) -> Result<String, String> {
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
