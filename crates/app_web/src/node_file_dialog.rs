//! Adaptation of the browser file-picker mechanism to the node widget port.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use node_graph::api::{
    DroppedFile, FileDialogError, FileDialogProgress, FileDialogRequest, FileDialogService,
};
use platform::{
    DroppedFileData, FileDialogFilter as HostFileDialogFilter, FilePickerError, FilePickerRequest,
    FilePickerService,
};

/// Tracks drag-and-drop imports, which read file contents asynchronously on
/// the web target (see `begin_async_drop_import`) but resolve synchronously
/// everywhere else. Completions are retrieved through the same
/// `take_picked`/`progress`/`cancel` polling already used for the picker
/// flow, keyed by the widget's `request_id`.
#[derive(Default)]
struct DropImportState {
    /// Request id to the generation that must still be current for its
    /// eventual result to be published, so a superseded drop is discarded.
    pending: HashMap<u64, u64>,
    completed: HashMap<u64, Result<String, FileDialogError>>,
    #[cfg(target_arch = "wasm32")]
    next_generation: u64,
    repaint: Option<Arc<dyn Fn() + Send + Sync>>,
}

pub(crate) struct BrowserNodeFileDialog {
    picker: Arc<Mutex<Box<dyn FilePickerService>>>,
    drops: Arc<Mutex<DropImportState>>,
}

impl BrowserNodeFileDialog {
    pub(crate) fn new(picker: Box<dyn FilePickerService>) -> Self {
        Self {
            picker: Arc::new(Mutex::new(picker)),
            drops: Arc::new(Mutex::new(DropImportState::default())),
        }
    }
}

impl FileDialogService for BrowserNodeFileDialog {
    fn available(&self, save: bool) -> bool {
        self.picker.lock().unwrap().available(save)
    }

    fn pick(&mut self, request: FileDialogRequest<'_>) -> Option<String> {
        let filters = request
            .filters
            .iter()
            .map(|filter| HostFileDialogFilter {
                name: filter.name.clone(),
                extensions: filter.extensions.clone(),
            })
            .collect::<Vec<_>>();
        self.picker
            .lock()
            .unwrap()
            .pick(FilePickerRequest {
                request_id: request.request_id,
                title: request.title,
                filters: &filters,
                save: request.save,
            })
            .map(|reference| reference.into_string())
    }

    fn take_picked(&mut self, request_id: u64) -> Option<Result<String, FileDialogError>> {
        if let Some(result) = self.drops.lock().unwrap().completed.remove(&request_id) {
            return Some(result);
        }
        self.picker.lock().unwrap().take_picked(request_id).map(|result| {
            result
                .map(|reference| reference.into_string())
                .map_err(FileDialogError::host)
        })
    }

    fn progress(&self, request_id: u64) -> Option<FileDialogProgress> {
        if self.drops.lock().unwrap().pending.contains_key(&request_id) {
            return Some(FileDialogProgress {
                completed_bytes: 0,
                total_bytes: None,
            });
        }
        self.picker
            .lock()
            .unwrap()
            .progress(request_id)
            .map(|progress| FileDialogProgress {
                completed_bytes: progress.completed_bytes,
                total_bytes: progress.total_bytes,
            })
    }

    fn cancel(&mut self, request_id: u64) -> bool {
        if self.drops.lock().unwrap().pending.remove(&request_id).is_some() {
            return true;
        }
        self.picker.lock().unwrap().cancel(request_id)
    }

    fn import_dropped(
        &mut self,
        request_id: u64,
        file: DroppedFile,
    ) -> Option<Result<String, FileDialogError>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = request_id;
            Some(match file.handle.bytes() {
                Ok(bytes) => finish_dropped_import(&self.picker, file.name, bytes),
                Err(message) => Err(dropped_read_failure(&file.name, message)),
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.begin_async_drop_import(request_id, file);
            None
        }
    }

    fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        let repaint: Arc<dyn Fn() + Send + Sync> = Arc::from(repaint);
        self.drops.lock().unwrap().repaint = Some(Arc::clone(&repaint));
        self.picker
            .lock()
            .unwrap()
            .set_repaint(Box::new(move || repaint()));
    }
}

#[cfg(target_arch = "wasm32")]
impl BrowserNodeFileDialog {
    fn begin_async_drop_import(&mut self, request_id: u64, file: DroppedFile) {
        let generation = {
            let mut drops = self.drops.lock().unwrap();
            drops.next_generation = drops.next_generation.wrapping_add(1).max(1);
            let generation = drops.next_generation;
            drops.pending.insert(request_id, generation);
            drops.completed.remove(&request_id);
            generation
        };
        let drops = Arc::clone(&self.drops);
        let picker = Arc::clone(&self.picker);
        let name = file.name;
        let handle = file.handle;
        wasm_bindgen_futures::spawn_local(async move {
            let result = match handle.bytes_async().await {
                Ok(bytes) => finish_dropped_import(&picker, name, bytes),
                Err(message) => Err(dropped_read_failure(&name, message)),
            };
            let mut drops = drops.lock().unwrap();
            if drops.pending.get(&request_id) == Some(&generation) {
                drops.pending.remove(&request_id);
                drops.completed.insert(request_id, result);
                if let Some(repaint) = &drops.repaint {
                    repaint();
                }
            }
        });
    }
}

/// Registers already-read bytes with the host picker.
fn finish_dropped_import(
    picker: &Arc<Mutex<Box<dyn FilePickerService>>>,
    name: String,
    bytes: Vec<u8>,
) -> Result<String, FileDialogError> {
    picker
        .lock()
        .unwrap()
        .import_dropped(DroppedFileData {
            name,
            path: None,
            bytes: Some(Arc::from(bytes)),
        })
        .map(|reference| reference.into_string())
        .map_err(FileDialogError::host)
}

/// Typed failure for a drop whose contents could not be read. The host
/// widget toolkit reports that failure as a bare message, so it is given the
/// same shape as an import failure here, where it enters this port.
fn dropped_read_failure(name: &str, message: String) -> FileDialogError {
    FileDialogError::host(FilePickerError::Read {
        name: name.to_owned(),
        message,
    })
}

#[cfg(test)]
mod node_file_dialog_tests {
    use std::error::Error as _;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use node_graph::api::{DroppedFile, FileDialogFilter, FileDialogRequest, FileDialogService};
    use platform::{
        DroppedFileData, FilePickerError, FilePickerProgress, FilePickerRequest, FilePickerService,
        FileReference,
    };

    use super::BrowserNodeFileDialog;

    #[derive(Debug)]
    struct FakeDroppedFile {
        path: PathBuf,
    }

    impl eframe::egui::DroppedFile for FakeDroppedFile {
        fn path(&self) -> &Path {
            &self.path
        }

        #[cfg(not(target_arch = "wasm32"))]
        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }

        #[cfg(target_arch = "wasm32")]
        fn bytes_async(
            &self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + '_>>
        {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[derive(Default)]
    struct PickerObservation {
        title: String,
        extensions: Vec<String>,
        dropped_name: String,
    }

    struct FakePicker {
        observation: Arc<Mutex<PickerObservation>>,
    }

    impl FilePickerService for FakePicker {
        fn available(&self, save: bool) -> bool {
            !save
        }

        fn pick(&mut self, request: FilePickerRequest<'_>) -> Option<FileReference> {
            let mut observation = self.observation.lock().unwrap();
            observation.title = request.title.to_owned();
            observation.extensions = request.filters[0].extensions.clone();
            Some(FileReference::from("host-file://selected".to_owned()))
        }

        fn take_picked(
            &mut self,
            request_id: u64,
        ) -> Option<Result<FileReference, FilePickerError>> {
            match request_id {
                7 => Some(Ok(FileReference::from("host-file://async".to_owned()))),
                8 => Some(Err(FilePickerError::Read {
                    name: "capture.sr".to_owned(),
                    message: "host read failed".to_owned(),
                })),
                _ => None,
            }
        }

        fn progress(&self, request_id: u64) -> Option<FilePickerProgress> {
            (request_id == 7).then_some(FilePickerProgress {
                completed_bytes: 4,
                total_bytes: Some(8),
            })
        }

        fn cancel(&mut self, request_id: u64) -> bool {
            request_id == 7
        }

        fn import_dropped(
            &mut self,
            file: DroppedFileData,
        ) -> Result<FileReference, FilePickerError> {
            self.observation.lock().unwrap().dropped_name = file.name;
            Ok(FileReference::from("host-file://dropped".to_owned()))
        }

        fn set_repaint(&mut self, _repaint: Box<dyn Fn() + Send + Sync>) {}
    }

    #[test]
    fn adapter_maps_widget_requests_to_the_generic_picker_contract() {
        let observation = Arc::new(Mutex::new(PickerObservation::default()));
        let mut dialog = BrowserNodeFileDialog::new(Box::new(FakePicker {
            observation: Arc::clone(&observation),
        }));
        let filters = vec![FileDialogFilter {
            name: "Capture".to_owned(),
            extensions: vec!["sr".to_owned()],
        }];

        assert_eq!(
            dialog.pick(FileDialogRequest {
                request_id: 7,
                title: "Select capture",
                filters: &filters,
                save: false,
            }),
            Some("host-file://selected".to_owned())
        );
        assert!(matches!(
            dialog.take_picked(7),
            Some(Ok(reference)) if reference == "host-file://async"
        ));
        let Some(Err(error)) = dialog.take_picked(8) else {
            panic!("host picker failure should cross the widget adapter");
        };
        assert!(error.source().unwrap().is::<FilePickerError>());
        assert_eq!(dialog.progress(7).unwrap().completed_bytes, 4);
        assert!(dialog.cancel(7));

        let handle: eframe::egui::DroppedFileHandle = Arc::new(FakeDroppedFile {
            path: PathBuf::from("capture.sr"),
        });
        assert_eq!(
            dialog
                .import_dropped(
                    42,
                    DroppedFile {
                        name: "capture.sr".to_owned(),
                        handle,
                    },
                )
                .expect("host target resolves the import synchronously")
                .unwrap(),
            "host-file://dropped"
        );

        let observation = observation.lock().unwrap();
        assert_eq!(observation.title, "Select capture");
        assert_eq!(observation.extensions, ["sr"]);
        assert_eq!(observation.dropped_name, "capture.sr");
    }
}
