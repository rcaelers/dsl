use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use js_sys::Uint8Array;
use wasm_bindgen_futures::JsFuture;

use node_graph::{DroppedFile, FileDialogRequest, FileDialogService};

use super::registry::{
    BrowserFileRegistry, IMPORT_CHUNK_BYTES, MAX_IMPORT_BYTES, file_limit_error,
};

#[derive(Default)]
struct DialogState {
    pending: HashSet<u64>,
    completed: HashMap<u64, Result<String, String>>,
    repaint: Option<Box<dyn Fn() + Send + Sync>>,
}

pub(crate) struct BrowserNodeFileDialogService {
    registry: Arc<BrowserFileRegistry>,
    state: Arc<Mutex<DialogState>>,
}

impl BrowserNodeFileDialogService {
    pub(crate) fn new(registry: Arc<BrowserFileRegistry>) -> Self {
        Self {
            registry,
            state: Arc::new(Mutex::new(DialogState::default())),
        }
    }
}

impl FileDialogService for BrowserNodeFileDialogService {
    fn available(&self, save: bool) -> bool {
        !save
    }

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        if request.save {
            return None;
        }
        {
            let mut state = self.state.lock().unwrap();
            if !state.pending.insert(request.request_id) {
                return None;
            }
        }

        let mut dialog = rfd::AsyncFileDialog::new();
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
        let registry = Arc::clone(&self.registry);
        let state = Arc::clone(&self.state);
        let request_id = request.request_id;
        wasm_bindgen_futures::spawn_local(async move {
            let selection = dialog.pick_file().await;
            let result = if let Some(file) = selection {
                let name = file.file_name();
                if file.inner().size() > MAX_IMPORT_BYTES as f64 {
                    Err(file_limit_error(&name))
                } else {
                    read_browser_file_chunks(file.inner())
                        .await
                        .and_then(|chunks| registry.register_chunks(name, chunks))
                }
            } else {
                let mut state = state.lock().unwrap();
                state.pending.remove(&request_id);
                if let Some(repaint) = &state.repaint {
                    repaint();
                }
                return;
            };
            let mut state = state.lock().unwrap();
            state.pending.remove(&request_id);
            state.completed.insert(request_id, result);
            if let Some(repaint) = &state.repaint {
                repaint();
            }
        });
        None
    }

    fn take_picked(&mut self, request_id: u64) -> Option<Result<String, String>> {
        self.state.lock().unwrap().completed.remove(&request_id)
    }

    fn import_dropped(&mut self, file: DroppedFile) -> Result<String, String> {
        let bytes = file
            .bytes
            .ok_or_else(|| format!("the browser did not provide bytes for '{}'", file.name))?;
        self.registry.register(file.name, bytes)
    }

    fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        self.state.lock().unwrap().repaint = Some(repaint);
    }
}

async fn read_browser_file_chunks(file: &web_sys::File) -> Result<Vec<Arc<[u8]>>, String> {
    let length = file.size();
    let mut chunks = Vec::new();
    let mut offset = 0_f64;
    while offset < length {
        let end = (offset + IMPORT_CHUNK_BYTES as f64).min(length);
        let blob = file
            .slice_with_f64_and_f64(offset, end)
            .map_err(|error| format!("could not slice browser file: {error:?}"))?;
        let buffer = JsFuture::from(blob.array_buffer())
            .await
            .map_err(|error| format!("could not read browser file: {error:?}"))?;
        chunks.push(Arc::from(Uint8Array::new(&buffer).to_vec()));
        offset = end;
    }
    Ok(chunks)
}
