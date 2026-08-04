use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast;

use logic_analyzer_ui::{HostCommand, HostService, HostUiCapabilities, OpenDialog, SaveDialog};

const MAX_GRAPH_DOCUMENT_BYTES: f64 = 32.0 * 1024.0 * 1024.0;

thread_local! {
    static OUTPUT_DOWNLOADS: RefCell<BrowserOutputDownloads> = RefCell::new(BrowserOutputDownloads::default());
    static OUTPUT_DOWNLOAD_REPAINT: RefCell<Option<Arc<dyn Fn() + Send + Sync>>> = const { RefCell::new(None) };
}

#[derive(Default)]
struct BrowserOutputDownloads {
    next_id: u64,
    files: HashMap<u64, BrowserOutputDownload>,
}

struct BrowserOutputDownload {
    name: String,
    bytes: Vec<u8>,
    producer_node: String,
    producer_socket: String,
}

#[derive(Clone)]
struct BrowserDocument {
    display_name: String,
    contents: Result<Arc<[u8]>, String>,
}

#[derive(Default)]
struct BrowserDocumentState {
    documents: HashMap<String, BrowserDocument>,
    commands: Vec<HostCommand>,
    repaint: Option<Arc<dyn Fn() + Send + Sync>>,
    next_reference: u64,
    open_pending: bool,
}

pub(crate) struct BrowserDocumentHostService {
    state: Arc<Mutex<BrowserDocumentState>>,
}

impl BrowserDocumentHostService {
    pub(crate) fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BrowserDocumentState::default())),
        }
    }

    fn reserve_document(&self, display_name: String) -> PathBuf {
        register_document(
            &mut self.state.lock().unwrap(),
            display_name,
            Ok(Arc::from([])),
        )
    }
}

/// Replaces the previous run's browser files with the latest completed run.
pub(crate) fn queue_output_files(
    files: impl IntoIterator<Item = super::web_output_storage::BrowserOutputFile>,
) {
    OUTPUT_DOWNLOADS.with(|downloads| {
        let mut downloads = downloads.borrow_mut();
        downloads.files.clear();
        for file in files {
            downloads.next_id = downloads.next_id.saturating_add(1);
            let id = downloads.next_id;
            downloads.files.insert(
                id,
                BrowserOutputDownload {
                    name: file.name,
                    bytes: file.bytes,
                    producer_node: file.producer_node,
                    producer_socket: file.producer_socket,
                },
            );
        }
    });
    OUTPUT_DOWNLOAD_REPAINT.with(|repaint| {
        if let Some(repaint) = repaint.borrow().as_ref() {
            repaint();
        }
    });
}

impl HostService for BrowserDocumentHostService {
    fn ui_capabilities(&self) -> HostUiCapabilities {
        HostUiCapabilities {
            direct_document_access: true,
            ..HostUiCapabilities::default()
        }
    }

    fn set_command_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        let repaint: Arc<dyn Fn() + Send + Sync> = repaint.into();
        self.state.lock().unwrap().repaint = Some(Arc::clone(&repaint));
        OUTPUT_DOWNLOAD_REPAINT.with(|output_repaint| {
            *output_repaint.borrow_mut() = Some(repaint);
        });
    }

    fn take_commands(&mut self) -> Vec<HostCommand> {
        std::mem::take(&mut self.state.lock().unwrap().commands)
    }

    fn pending_output_downloads(&self) -> Vec<logic_analyzer_ui::DownloadableOutput> {
        OUTPUT_DOWNLOADS.with(|downloads| {
            let mut downloads = downloads
                .borrow()
                .files
                .iter()
                .map(|(&id, file)| logic_analyzer_ui::DownloadableOutput {
                    id,
                    name: file.name.clone(),
                    producer_node: file.producer_node.clone(),
                    producer_socket: file.producer_socket.clone(),
                    byte_len: file.bytes.len() as u64,
                })
                .collect::<Vec<_>>();
            downloads.sort_by_key(|output| output.id);
            downloads
        })
    }

    fn download_output(&mut self, id: u64) -> Result<(), String> {
        let output = OUTPUT_DOWNLOADS.with(|downloads| downloads.borrow_mut().files.remove(&id));
        let Some(output) = output else {
            return Err("that output is no longer available".to_owned());
        };
        if let Err(error) = download_file(&output.name, &output.bytes, "application/octet-stream") {
            OUTPUT_DOWNLOADS.with(|downloads| {
                downloads.borrow_mut().files.insert(id, output);
            });
            return Err(error);
        }
        Ok(())
    }

    fn document_exists(&self, path: &Path) -> bool {
        self.state
            .lock()
            .unwrap()
            .documents
            .contains_key(&path.to_string_lossy().into_owned())
    }

    fn document_display_name(&self, path: &Path) -> String {
        self.state
            .lock()
            .unwrap()
            .documents
            .get(&path.to_string_lossy().into_owned())
            .map(|document| document.display_name.clone())
            .unwrap_or_else(|| path.display().to_string())
    }

    fn choose_open_file(&mut self, request: OpenDialog<'_>) -> Option<PathBuf> {
        {
            let mut state = self.state.lock().unwrap();
            if state.open_pending {
                return None;
            }
            state.open_pending = true;
        }
        let mut dialog = rfd::AsyncFileDialog::new().set_title(request.title);
        dialog = dialog.add_filter(request.filter_label, request.extensions);
        let state = Arc::clone(&self.state);
        wasm_bindgen_futures::spawn_local(async move {
            let selected = dialog.pick_file().await;
            let selected = if let Some(file) = selected {
                let display_name = file.file_name();
                let contents = if file.inner().size() > MAX_GRAPH_DOCUMENT_BYTES {
                    Err(format!(
                        "'{display_name}' is too large to be a graph document (32 MiB limit)"
                    ))
                } else {
                    Ok(Arc::from(file.read().await))
                };
                Some((display_name, contents))
            } else {
                None
            };
            let repaint = {
                let mut state = state.lock().unwrap();
                state.open_pending = false;
                if let Some((display_name, contents)) = selected {
                    let path = register_document(&mut state, display_name, contents);
                    state.commands.push(HostCommand::LoadPath(path));
                }
                state.repaint.clone()
            };
            if let Some(repaint) = repaint {
                repaint();
            }
        });
        None
    }

    fn choose_save_file(&mut self, request: SaveDialog<'_>) -> Option<PathBuf> {
        let suggested = ensure_extension(request.default_file_name, request.extensions.first());
        let display_name = web_sys::window()?
            .prompt_with_message_and_default(request.title, &suggested)
            .ok()??;
        let display_name = ensure_extension(display_name.trim(), request.extensions.first());
        (!display_name.is_empty()).then(|| self.reserve_document(display_name))
    }

    fn choose_directory(&mut self) -> Option<PathBuf> {
        None
    }

    fn load_graph(&mut self, path: &Path) -> Result<node_graph::GraphState, String> {
        let key = path.to_string_lossy().into_owned();
        let document = self
            .state
            .lock()
            .unwrap()
            .documents
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "browser graph '{}' is no longer available; open the file again",
                    path.display()
                )
            })?;
        let contents = document.contents?;
        serde_json::from_slice(&contents)
            .map_err(|error| format!("could not parse {}: {error}", document.display_name))
    }

    fn save_graph(&mut self, path: &Path, graph: &serde_json::Value) -> Result<(), String> {
        let contents = serde_json::to_vec_pretty(graph)
            .map_err(|error| format!("could not serialize graph: {error}"))?;
        let key = path.to_string_lossy().into_owned();
        let display_name = {
            let mut state = self.state.lock().unwrap();
            let document = state.documents.get_mut(&key).ok_or_else(|| {
                format!(
                    "browser graph '{}' is no longer available; use Save As",
                    path.display()
                )
            })?;
            document.contents = Ok(Arc::from(contents.clone()));
            document.display_name.clone()
        };
        download_file(&display_name, &contents, "application/json")
    }
}

fn register_document(
    state: &mut BrowserDocumentState,
    display_name: String,
    contents: Result<Arc<[u8]>, String>,
) -> PathBuf {
    state.next_reference = state.next_reference.saturating_add(1);
    let safe_name = display_name.replace(['/', '\\'], "_");
    let reference = format!("browser-document://{}/{safe_name}", state.next_reference);
    state.documents.insert(
        reference.clone(),
        BrowserDocument {
            display_name,
            contents,
        },
    );
    PathBuf::from(reference)
}

fn ensure_extension(name: &str, extension: Option<&&str>) -> String {
    let mut name = name.replace(['/', '\\'], "_");
    if Path::new(&name).extension().is_none()
        && let Some(extension) = extension
    {
        name.push('.');
        name.push_str(extension);
    }
    name
}

/// Starts a browser download from in-memory bytes.
pub(crate) fn download_file(
    display_name: &str,
    contents: &[u8],
    media_type: &str,
) -> Result<(), String> {
    let array = js_sys::Array::new();
    let bytes = js_sys::Uint8Array::from(contents);
    array.push(&bytes.buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(media_type);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&array, &options)
        .map_err(|error| format!("could not create graph download: {error:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|error| format!("could not create graph download URL: {error:?}"))?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "the browser document is unavailable".to_owned())?;
        let anchor: web_sys::HtmlAnchorElement = document
            .create_element("a")
            .map_err(|error| format!("could not create graph download link: {error:?}"))?
            .dyn_into()
            .map_err(|_| "could not create graph download link".to_owned())?;
        anchor.set_href(&url);
        anchor.set_download(display_name);
        let body = document
            .body()
            .ok_or_else(|| "the browser document has no body".to_owned())?;
        body.append_child(&anchor)
            .map_err(|error| format!("could not attach graph download: {error:?}"))?;
        anchor.click();
        body.remove_child(&anchor)
            .map_err(|error| format!("could not detach graph download: {error:?}"))?;
        Ok(())
    })();
    web_sys::Url::revoke_object_url(&url)
        .map_err(|error| format!("could not release graph download URL: {error:?}"))?;
    result
}

#[cfg(test)]
mod web_document_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test(unsupported = test)]
    fn registered_browser_documents_load_by_opaque_reference() {
        let mut state = BrowserDocumentState::default();
        let contents = serde_json::to_vec(&node_graph::GraphState::default()).unwrap();
        let path = register_document(
            &mut state,
            "example.json".to_owned(),
            Ok(Arc::from(contents)),
        );
        let mut service = BrowserDocumentHostService {
            state: Arc::new(Mutex::new(state)),
        };

        assert!(service.document_exists(&path));
        assert_eq!(service.document_display_name(&path), "example.json");
        assert!(service.load_graph(&path).is_ok());
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn save_names_are_sanitized_and_gain_the_requested_extension() {
        assert_eq!(ensure_extension("demo", Some(&"json")), "demo.json");
        assert_eq!(
            ensure_extension("folder/demo.json", Some(&"json")),
            "folder_demo.json"
        );
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn graph_outputs_keep_only_the_latest_completed_run() {
        let service = BrowserDocumentHostService::new();
        queue_output_files([super::super::web_output_storage::BrowserOutputFile {
            name: "capture.bin".to_owned(),
            bytes: vec![1, 2, 3],
            producer_node: "Parallel Decoder".to_owned(),
            producer_socket: "Words".to_owned(),
        }]);

        let downloads = service.pending_output_downloads();
        assert_eq!(downloads.len(), 1);
        assert_eq!(downloads[0].name, "capture.bin");
        assert_eq!(downloads[0].byte_len, 3);
        assert_eq!(downloads[0].producer_node, "Parallel Decoder");
        assert_eq!(downloads[0].producer_socket, "Words");
        assert_eq!(service.pending_output_downloads()[0].id, downloads[0].id);

        queue_output_files([super::super::web_output_storage::BrowserOutputFile {
            name: "latest.csv".to_owned(),
            bytes: vec![4, 5],
            producer_node: "UART Decoder".to_owned(),
            producer_socket: "Frames".to_owned(),
        }]);
        let latest = service.pending_output_downloads();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].name, "latest.csv");
        assert_eq!(latest[0].producer_node, "UART Decoder");

        queue_output_files(Vec::new());
        assert!(service.pending_output_downloads().is_empty());
    }
}
