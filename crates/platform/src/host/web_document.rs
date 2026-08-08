//! Browser document selection, byte storage, and downloads.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use wasm_bindgen::JsCast;

use crate::{DocumentError, FileOpenDialog, FileSaveDialog};

const MAX_DOCUMENT_BYTES: f64 = 32.0 * 1024.0 * 1024.0;

thread_local! {
    static OUTPUT_DOWNLOADS: RefCell<BrowserOutputDownloads> = RefCell::new(BrowserOutputDownloads::default());
    static OUTPUT_DOWNLOAD_REPAINT: RefCell<Option<Arc<dyn Fn() + Send + Sync>>> = const { RefCell::new(None) };
}

/// One completed in-memory output available for explicit download.
pub struct BrowserDownload {
    pub id: u64,
    pub name: String,
    pub annotations: Vec<String>,
    pub byte_len: u64,
}

/// Bytes and opaque annotations queued for an explicit browser download.
pub struct BrowserDownloadFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub annotations: Vec<String>,
}

#[derive(Default)]
struct BrowserOutputDownloads {
    next_id: u64,
    files: HashMap<u64, BrowserOutputDownload>,
}

struct BrowserOutputDownload {
    name: String,
    bytes: Vec<u8>,
    annotations: Vec<String>,
}

struct BrowserDocument {
    display_name: String,
    contents: BrowserDocumentContents,
}

enum BrowserDocumentContents {
    Available(Arc<[u8]>),
    TooLarge { max_bytes: u64 },
}

#[derive(Default)]
struct BrowserDocumentState {
    documents: HashMap<String, BrowserDocument>,
    opened_documents: Vec<PathBuf>,
    repaint: Option<Arc<dyn Fn() + Send + Sync>>,
    next_reference: u64,
    open_pending: bool,
}

/// Browser implementation of neutral document and download mechanisms.
pub struct BrowserDocumentHost {
    state: Arc<Mutex<BrowserDocumentState>>,
}

impl BrowserDocumentHost {
    /// Creates an empty browser document host.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(BrowserDocumentState::default())),
        }
    }

    /// Installs the callback used after an asynchronous host event.
    pub fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        let repaint: Arc<dyn Fn() + Send + Sync> = repaint.into();
        self.state.lock().unwrap().repaint = Some(Arc::clone(&repaint));
        OUTPUT_DOWNLOAD_REPAINT.with(|output_repaint| {
            *output_repaint.borrow_mut() = Some(repaint);
        });
    }

    /// Drains document references selected by completed asynchronous dialogs.
    pub fn take_opened_documents(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.state.lock().unwrap().opened_documents)
    }

    /// Reports whether a browser-owned document reference remains available.
    pub fn document_exists(&self, path: &Path) -> bool {
        self.state
            .lock()
            .unwrap()
            .documents
            .contains_key(&path.to_string_lossy().into_owned())
    }

    /// Returns the user-facing name for a browser-owned document reference.
    pub fn document_display_name(&self, path: &Path) -> String {
        self.state
            .lock()
            .unwrap()
            .documents
            .get(&path.to_string_lossy().into_owned())
            .map(|document| document.display_name.clone())
            .unwrap_or_else(|| path.display().to_string())
    }

    /// Starts an asynchronous browser file selection.
    pub fn choose_open_file(&mut self, request: FileOpenDialog) -> Option<PathBuf> {
        {
            let mut state = self.state.lock().unwrap();
            if state.open_pending {
                return None;
            }
            state.open_pending = true;
        }
        let mut dialog = rfd::AsyncFileDialog::new().set_title(&request.title);
        for filter in &request.filters {
            let extensions = filter
                .extensions
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            dialog = dialog.add_filter(&filter.name, &extensions);
        }
        let state = Arc::clone(&self.state);
        wasm_bindgen_futures::spawn_local(async move {
            let selected = dialog.pick_file().await;
            let selected = if let Some(file) = selected {
                let display_name = file.file_name();
                let contents = if file.inner().size() > MAX_DOCUMENT_BYTES {
                    BrowserDocumentContents::TooLarge {
                        max_bytes: MAX_DOCUMENT_BYTES as u64,
                    }
                } else {
                    BrowserDocumentContents::Available(Arc::from(file.read().await))
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
                    state.opened_documents.push(path);
                }
                state.repaint.clone()
            };
            if let Some(repaint) = repaint {
                repaint();
            }
        });
        None
    }

    /// Reserves a browser-owned document destination with the selected name.
    pub fn choose_save_file(&mut self, request: FileSaveDialog) -> Option<PathBuf> {
        let extension = request
            .filters
            .first()
            .and_then(|filter| filter.extensions.first())
            .map(String::as_str);
        let suggested = ensure_extension(&request.default_file_name, extension);
        let display_name = web_sys::window()?
            .prompt_with_message_and_default(&request.title, &suggested)
            .ok()??;
        let display_name = ensure_extension(display_name.trim(), extension);
        (!display_name.is_empty()).then(|| {
            register_document(
                &mut self.state.lock().unwrap(),
                display_name,
                BrowserDocumentContents::Available(Arc::from([])),
            )
        })
    }

    /// Reads bytes from a browser-owned document reference.
    pub fn read_document(&self, path: &Path) -> Result<Vec<u8>, DocumentError> {
        let key = path.to_string_lossy().into_owned();
        let state = self.state.lock().unwrap();
        let document = state
            .documents
            .get(&key)
            .ok_or_else(|| DocumentError::Unavailable {
                path: path.to_owned(),
            })?;
        match &document.contents {
            BrowserDocumentContents::Available(contents) => Ok(contents.to_vec()),
            BrowserDocumentContents::TooLarge { max_bytes } => Err(DocumentError::TooLarge {
                name: document.display_name.clone(),
                max_bytes: *max_bytes,
            }),
        }
    }

    /// Stores and downloads bytes for a browser-owned document destination.
    pub fn write_document(&mut self, path: &Path, contents: &[u8]) -> Result<(), DocumentError> {
        let key = path.to_string_lossy().into_owned();
        let display_name = {
            let mut state = self.state.lock().unwrap();
            let document =
                state
                    .documents
                    .get_mut(&key)
                    .ok_or_else(|| DocumentError::Unavailable {
                        path: path.to_owned(),
                    })?;
            document.contents = BrowserDocumentContents::Available(Arc::from(contents));
            document.display_name.clone()
        };
        download_file(&display_name, contents, "application/octet-stream")
            .map_err(|error| DocumentError::write(path, std::io::Error::other(error)))
    }

    /// Lists completed output bytes retained for explicit download.
    pub fn pending_downloads(&self) -> Vec<BrowserDownload> {
        OUTPUT_DOWNLOADS.with(|downloads| {
            let mut downloads = downloads
                .borrow()
                .files
                .iter()
                .map(|(&id, file)| BrowserDownload {
                    id,
                    name: file.name.clone(),
                    annotations: file.annotations.clone(),
                    byte_len: file.bytes.len() as u64,
                })
                .collect::<Vec<_>>();
            downloads.sort_by_key(|download| download.id);
            downloads
        })
    }

    /// Downloads and removes one retained output.
    pub fn download(&mut self, id: u64) -> Result<(), String> {
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
}

impl Default for BrowserDocumentHost {
    fn default() -> Self {
        Self::new()
    }
}

/// Replaces the page's pending download queue with host-agnostic byte payloads.
pub fn queue_browser_downloads(files: impl IntoIterator<Item = BrowserDownloadFile>) {
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
                    annotations: file.annotations,
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

fn register_document(
    state: &mut BrowserDocumentState,
    display_name: String,
    contents: BrowserDocumentContents,
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

fn ensure_extension(name: &str, extension: Option<&str>) -> String {
    let mut name = name.replace(['/', '\\'], "_");
    if Path::new(&name).extension().is_none()
        && let Some(extension) = extension
    {
        name.push('.');
        name.push_str(extension);
    }
    name
}

fn download_file(display_name: &str, contents: &[u8], media_type: &str) -> Result<(), String> {
    let array = js_sys::Array::new();
    let bytes = js_sys::Uint8Array::from(contents);
    array.push(&bytes.buffer());
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(media_type);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&array, &options)
        .map_err(|error| format!("could not create download: {error:?}"))?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)
        .map_err(|error| format!("could not create download URL: {error:?}"))?;
    let result = (|| {
        let document = web_sys::window()
            .and_then(|window| window.document())
            .ok_or_else(|| "the browser document is unavailable".to_owned())?;
        let anchor: web_sys::HtmlAnchorElement = document
            .create_element("a")
            .map_err(|error| format!("could not create download link: {error:?}"))?
            .dyn_into()
            .map_err(|_| "could not create download link".to_owned())?;
        anchor.set_href(&url);
        anchor.set_download(display_name);
        let body = document
            .body()
            .ok_or_else(|| "the browser document has no body".to_owned())?;
        body.append_child(&anchor)
            .map_err(|error| format!("could not attach download: {error:?}"))?;
        anchor.click();
        body.remove_child(&anchor)
            .map_err(|error| format!("could not detach download: {error:?}"))?;
        Ok(())
    })();
    web_sys::Url::revoke_object_url(&url)
        .map_err(|error| format!("could not release download URL: {error:?}"))?;
    result
}

#[cfg(test)]
mod web_document_tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    #[wasm_bindgen_test(unsupported = test)]
    fn registered_documents_load_by_opaque_reference() {
        let mut state = BrowserDocumentState::default();
        let path = register_document(
            &mut state,
            "example.json".to_owned(),
            BrowserDocumentContents::Available(Arc::from(&b"{}"[..])),
        );
        let service = BrowserDocumentHost {
            state: Arc::new(Mutex::new(state)),
        };

        assert!(service.document_exists(&path));
        assert_eq!(service.document_display_name(&path), "example.json");
        assert_eq!(service.read_document(&path).unwrap(), b"{}");
    }

    #[wasm_bindgen_test(unsupported = test)]
    fn save_names_are_sanitized_and_gain_the_requested_extension() {
        assert_eq!(ensure_extension("demo", Some("json")), "demo.json");
        assert_eq!(
            ensure_extension("folder/demo.json", Some("json")),
            "folder_demo.json"
        );
    }
}
