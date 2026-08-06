use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use js_sys::Uint8Array;
use wasm_bindgen_futures::JsFuture;

use logic_analyzer_platform::{
    DroppedFileData, FilePickerProgress, FilePickerRequest, FilePickerService, FileReference,
};
use platform_artifacts::SourceIdentity;

use super::super::web_capture_worker::{attach_capture_file, cancel_capture_file_attachment};
use super::registry::{
    BrowserFileRegistry, IMPORT_CHUNK_BYTES, MAX_IMPORT_BYTES, file_limit_error,
};

#[derive(Default)]
struct DialogState {
    pending: HashMap<u64, u64>,
    completed: HashMap<u64, Result<FileReference, String>>,
    progress: HashMap<u64, FilePickerProgress>,
    attachments: HashMap<u64, String>,
    next_generation: u64,
    repaint: Option<Arc<dyn Fn() + Send + Sync>>,
}

pub(crate) struct BrowserFilePickerService {
    registry: Arc<BrowserFileRegistry>,
    state: Arc<Mutex<DialogState>>,
}

impl BrowserFilePickerService {
    pub(crate) fn new(registry: Arc<BrowserFileRegistry>) -> Self {
        Self {
            registry,
            state: Arc::new(Mutex::new(DialogState::default())),
        }
    }
}

impl FilePickerService for BrowserFilePickerService {
    fn available(&self, save: bool) -> bool {
        !save
    }

    fn pick(&mut self, request: FilePickerRequest<'_>) -> Option<FileReference> {
        if request.save {
            return None;
        }
        let generation = {
            let mut state = self.state.lock().unwrap();
            if state.pending.contains_key(&request.request_id) {
                return None;
            }
            state.next_generation = state.next_generation.wrapping_add(1).max(1);
            let generation = state.next_generation;
            state.pending.insert(request.request_id, generation);
            state.completed.remove(&request.request_id);
            state.progress.insert(
                request.request_id,
                FilePickerProgress {
                    completed_bytes: 0,
                    total_bytes: None,
                },
            );
            generation
        };
        request_repaint(&self.state);

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
                let length = file.inner().size();
                if !update_progress(
                    &state,
                    request_id,
                    generation,
                    FilePickerProgress {
                        completed_bytes: 0,
                        total_bytes: Some(length as u64),
                    },
                ) {
                    None
                } else {
                    let reference = registry.allocate_reference(&name);
                    let progress_state = Arc::clone(&state);
                    let progress_reference = reference.clone();
                    let complete_state = Arc::clone(&state);
                    let complete_registry = Arc::clone(&registry);
                    let complete_reference = reference.clone();
                    let complete_name = name.clone();
                    match attach_capture_file(
                        &reference,
                        &name,
                        file.inner(),
                        Box::new(move |completed_bytes, total_bytes| {
                            if !update_progress(
                                &progress_state,
                                request_id,
                                generation,
                                FilePickerProgress {
                                    completed_bytes,
                                    total_bytes: Some(total_bytes),
                                },
                            ) {
                                cancel_capture_file_attachment(&progress_reference);
                            }
                        }),
                        Box::new(move |result| {
                            let result = result.and_then(|attached| {
                                complete_registry.register_worker_backed(
                                    complete_reference.clone(),
                                    complete_name,
                                    length as u64,
                                    attached.identity,
                                    attached.metadata,
                                )?;
                                Ok(FileReference::from(complete_reference))
                            });
                            finish_request(&complete_state, request_id, generation, Some(result));
                        }),
                    ) {
                        Ok(true) => {
                            state
                                .lock()
                                .unwrap()
                                .attachments
                                .insert(request_id, reference);
                            return;
                        }
                        Ok(false) => {}
                        Err(error) => tracing::warn!(
                            %error,
                            "browser capture worker did not accept the selected file"
                        ),
                    }
                    if length > MAX_IMPORT_BYTES as f64 {
                        Some(Err(file_limit_error(&name)))
                    } else {
                        match read_browser_file_chunks(file.inner(), &state, request_id, generation)
                            .await
                        {
                            Ok(Some((chunks, identity))) => Some(
                                registry
                                    .register_chunks_with_identity(name, chunks, identity)
                                    .map(FileReference::from),
                            ),
                            Ok(None) => None,
                            Err(error) => Some(Err(error)),
                        }
                    }
                }
            } else {
                None
            };
            finish_request(&state, request_id, generation, result);
        });
        None
    }

    fn take_picked(&mut self, request_id: u64) -> Option<Result<FileReference, String>> {
        self.state.lock().unwrap().completed.remove(&request_id)
    }

    fn progress(&self, request_id: u64) -> Option<FilePickerProgress> {
        self.state
            .lock()
            .unwrap()
            .progress
            .get(&request_id)
            .copied()
    }

    fn cancel(&mut self, request_id: u64) -> bool {
        let (cancelled, attachment) = {
            let mut state = self.state.lock().unwrap();
            let cancelled = state.pending.remove(&request_id).is_some();
            if cancelled {
                state.progress.remove(&request_id);
            }
            (cancelled, state.attachments.remove(&request_id))
        };
        if let Some(reference) = attachment {
            cancel_capture_file_attachment(&reference);
        }
        if cancelled {
            request_repaint(&self.state);
        }
        cancelled
    }

    fn import_dropped(&mut self, file: DroppedFileData) -> Result<FileReference, String> {
        let bytes = file
            .bytes
            .ok_or_else(|| format!("the browser did not provide bytes for '{}'", file.name))?;
        self.registry
            .register(file.name, bytes)
            .map(FileReference::from)
    }

    fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        self.state.lock().unwrap().repaint = Some(Arc::from(repaint));
    }
}

async fn read_browser_file_chunks(
    file: &web_sys::File,
    state: &Arc<Mutex<DialogState>>,
    request_id: u64,
    generation: u64,
) -> Result<Option<(Vec<Arc<[u8]>>, SourceIdentity)>, String> {
    let length = file.size();
    let mut chunks = Vec::new();
    let mut hasher = blake3::Hasher::new();
    let mut offset = 0_f64;
    while offset < length {
        if !request_is_active(state, request_id, generation) {
            return Ok(None);
        }
        let end = (offset + IMPORT_CHUNK_BYTES as f64).min(length);
        let blob = file
            .slice_with_f64_and_f64(offset, end)
            .map_err(|error| format!("could not slice browser file: {error:?}"))?;
        let buffer = JsFuture::from(blob.array_buffer())
            .await
            .map_err(|error| format!("could not read browser file: {error:?}"))?;
        let bytes = Uint8Array::new(&buffer).to_vec();
        hasher.update(&bytes);
        chunks.push(Arc::from(bytes));
        offset = end;
        if !update_progress(
            state,
            request_id,
            generation,
            FilePickerProgress {
                completed_bytes: offset as u64,
                total_bytes: Some(length as u64),
            },
        ) {
            return Ok(None);
        }
    }
    let identity = SourceIdentity::from_bytes(*hasher.finalize().as_bytes());
    Ok(Some((chunks, identity)))
}

fn request_is_active(state: &Arc<Mutex<DialogState>>, request_id: u64, generation: u64) -> bool {
    state.lock().unwrap().pending.get(&request_id) == Some(&generation)
}

fn update_progress(
    state: &Arc<Mutex<DialogState>>,
    request_id: u64,
    generation: u64,
    progress: FilePickerProgress,
) -> bool {
    let updated = {
        let mut state = state.lock().unwrap();
        if state.pending.get(&request_id) != Some(&generation) {
            return false;
        }
        state.progress.insert(request_id, progress);
        true
    };
    request_repaint(state);
    updated
}

fn finish_request(
    state: &Arc<Mutex<DialogState>>,
    request_id: u64,
    generation: u64,
    result: Option<Result<FileReference, String>>,
) {
    let finished = {
        let mut state = state.lock().unwrap();
        if state.pending.get(&request_id) != Some(&generation) {
            return;
        }
        state.pending.remove(&request_id);
        state.progress.remove(&request_id);
        state.attachments.remove(&request_id);
        if let Some(result) = result {
            state.completed.insert(request_id, result);
        }
        true
    };
    if finished {
        request_repaint(state);
    }
}

fn request_repaint(state: &Arc<Mutex<DialogState>>) {
    let repaint = state.lock().unwrap().repaint.clone();
    if let Some(repaint) = repaint {
        repaint();
    }
}

#[cfg(test)]
mod dialog_tests {
    use std::sync::Arc;

    use wasm_bindgen_test::wasm_bindgen_test;

    use logic_analyzer_platform::{FilePickerProgress, FilePickerService, FileReference};

    use super::{BrowserFilePickerService, BrowserFileRegistry, finish_request};

    #[wasm_bindgen_test(unsupported = test)]
    fn cancelled_generation_cannot_publish_over_a_new_request() {
        let mut dialog = BrowserFilePickerService::new(Arc::new(BrowserFileRegistry::default()));
        {
            let mut state = dialog.state.lock().unwrap();
            state.pending.insert(7, 1);
            state.progress.insert(
                7,
                FilePickerProgress {
                    completed_bytes: 4,
                    total_bytes: Some(8),
                },
            );
        }

        assert!(dialog.cancel(7));
        assert_eq!(dialog.progress(7), None);
        {
            let mut state = dialog.state.lock().unwrap();
            state.pending.insert(7, 2);
            state.progress.insert(
                7,
                FilePickerProgress {
                    completed_bytes: 0,
                    total_bytes: None,
                },
            );
        }

        finish_request(
            &dialog.state,
            7,
            1,
            Some(Ok(FileReference::from("browser-file://stale".to_owned()))),
        );

        assert_eq!(dialog.take_picked(7), None);
        assert_eq!(
            dialog.progress(7),
            Some(FilePickerProgress {
                completed_bytes: 0,
                total_bytes: None,
            })
        );
    }
}
