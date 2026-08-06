//! Browser adaptation of neutral document mechanisms to UI-owned ports.

use std::path::{Path, PathBuf};

use logic_analyzer_platform::{
    BrowserDocumentHost, FileDialogFilter, FileOpenDialog, FileSaveDialog,
};
use logic_analyzer_ui::{
    DownloadableOutput, HostCommand, HostService, HostUiCapabilities, OpenDialog, SaveDialog,
};

pub(crate) struct BrowserHostService {
    documents: BrowserDocumentHost,
}

impl BrowserHostService {
    pub(crate) fn new() -> Self {
        Self {
            documents: BrowserDocumentHost::new(),
        }
    }
}

impl HostService for BrowserHostService {
    fn ui_capabilities(&self) -> HostUiCapabilities {
        HostUiCapabilities {
            direct_document_access: true,
            ..HostUiCapabilities::default()
        }
    }

    fn set_command_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        self.documents.set_repaint(repaint);
    }

    fn take_commands(&mut self) -> Vec<HostCommand> {
        self.documents
            .take_opened_documents()
            .into_iter()
            .map(HostCommand::LoadPath)
            .collect()
    }

    fn pending_output_downloads(&self) -> Vec<DownloadableOutput> {
        self.documents
            .pending_downloads()
            .into_iter()
            .map(|download| {
                let mut annotations = download.annotations.into_iter();
                DownloadableOutput {
                    id: download.id,
                    name: download.name,
                    producer_node: annotations.next().unwrap_or_default(),
                    producer_socket: annotations.next().unwrap_or_default(),
                    byte_len: download.byte_len,
                }
            })
            .collect()
    }

    fn download_output(&mut self, id: u64) -> Result<(), String> {
        self.documents.download(id)
    }

    fn document_exists(&self, path: &Path) -> bool {
        self.documents.document_exists(path)
    }

    fn document_display_name(&self, path: &Path) -> String {
        self.documents.document_display_name(path)
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

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String> {
        let contents = self.documents.read_document(path)?;
        serde_json::from_slice(&contents)
            .map_err(|error| format!("could not parse {}: {error}", path.display()))
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(graph)
            .map_err(|error| format!("could not serialize graph: {error}"))?;
        self.documents.write_document(path, &contents)
    }
}
