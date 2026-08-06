//! Adaptation of the browser file-picker mechanism to the node widget port.

use node_graph::{DroppedFile, FileDialogProgress, FileDialogRequest, FileDialogService};
use platform::{
    DroppedFileData, FileDialogFilter as HostFileDialogFilter, FilePickerRequest, FilePickerService,
};

pub(crate) struct BrowserNodeFileDialog {
    picker: Box<dyn FilePickerService>,
}

impl BrowserNodeFileDialog {
    pub(crate) fn new(picker: Box<dyn FilePickerService>) -> Self {
        Self { picker }
    }
}

impl FileDialogService for BrowserNodeFileDialog {
    fn available(&self, save: bool) -> bool {
        self.picker.available(save)
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
            .pick(FilePickerRequest {
                request_id: request.request_id,
                title: request.title,
                filters: &filters,
                save: request.save,
            })
            .map(|reference| reference.into_string())
    }

    fn take_picked(&mut self, request_id: u64) -> Option<Result<String, String>> {
        self.picker
            .take_picked(request_id)
            .map(|result| result.map(|reference| reference.into_string()))
    }

    fn progress(&self, request_id: u64) -> Option<FileDialogProgress> {
        self.picker
            .progress(request_id)
            .map(|progress| FileDialogProgress {
                completed_bytes: progress.completed_bytes,
                total_bytes: progress.total_bytes,
            })
    }

    fn cancel(&mut self, request_id: u64) -> bool {
        self.picker.cancel(request_id)
    }

    fn import_dropped(&mut self, file: DroppedFile) -> Result<String, String> {
        self.picker
            .import_dropped(DroppedFileData {
                name: file.name,
                path: file.path,
                bytes: file.bytes,
            })
            .map(|reference| reference.into_string())
    }

    fn set_repaint(&mut self, repaint: Box<dyn Fn() + Send + Sync>) {
        self.picker.set_repaint(repaint);
    }
}

#[cfg(test)]
mod node_file_dialog_tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use node_graph::{DroppedFile, FileDialogFilter, FileDialogRequest, FileDialogService};
    use platform::{
        DroppedFileData, FilePickerProgress, FilePickerRequest, FilePickerService, FileReference,
    };

    use super::BrowserNodeFileDialog;

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

        fn take_picked(&mut self, request_id: u64) -> Option<Result<FileReference, String>> {
            (request_id == 7).then(|| Ok(FileReference::from("host-file://async".to_owned())))
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

        fn import_dropped(&mut self, file: DroppedFileData) -> Result<FileReference, String> {
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
        assert_eq!(
            dialog.take_picked(7),
            Some(Ok("host-file://async".to_owned()))
        );
        assert_eq!(dialog.progress(7).unwrap().completed_bytes, 4);
        assert!(dialog.cancel(7));
        assert_eq!(
            dialog
                .import_dropped(DroppedFile {
                    name: "capture.sr".to_owned(),
                    path: Some(PathBuf::from("capture.sr")),
                    bytes: None,
                })
                .unwrap(),
            "host-file://dropped"
        );

        let observation = observation.lock().unwrap();
        assert_eq!(observation.title, "Select capture");
        assert_eq!(observation.extensions, ["sr"]);
        assert_eq!(observation.dropped_name, "capture.sr");
    }
}
