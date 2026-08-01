use std::fmt;

use egui::{Rect, Ui};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileDialogFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

pub struct FileDialogRequest<'a> {
    pub title: &'a str,
    pub filters: &'a [FileDialogFilter],
    pub save: bool,
}

pub trait FileDialogService: Send {
    fn available(&self) -> bool;

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String>;
}

pub struct InlineControlContext<'a> {
    file_dialog: &'a mut dyn FileDialogService,
}

impl<'a> InlineControlContext<'a> {
    pub(crate) fn new(file_dialog: &'a mut dyn FileDialogService) -> Self {
        Self { file_dialog }
    }

    pub fn file_dialog_available(&self) -> bool {
        self.file_dialog.available()
    }

    pub fn pick_file(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        self.file_dialog.pick(request)
    }
}

pub(crate) struct UnavailableFileDialogService;

impl FileDialogService for UnavailableFileDialogService {
    fn available(&self) -> bool {
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
