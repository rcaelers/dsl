//! Native composition of Sigrok discovery with the UI node-catalog port.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

use logic_analyzer_protocol_decoders::sigrok_decoder::{
    SigrokCatalogError, SigrokCatalogScanner, SigrokCatalogSnapshot,
};
use logic_analyzer_ui::{NodeCatalogService, NodeCatalogSnapshot};
use node_graph::api::NodeTemplate;
use platform::{DocumentError, NativeDocumentHost};
use platform_runtime::{WorkExecutor, WorkTask};

const NAMESPACE: &str = "logic_conduit.sigrok_python";

#[derive(Debug, Default, Deserialize, Serialize)]
struct SavedSettings {
    directories: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
enum SigrokCatalogSettingsError {
    #[error("could not read Sigrok decoder settings from {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: DocumentError,
    },
    #[error("could not decode Sigrok decoder settings from {}: {source}", path.display())]
    Decode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("could not create the Sigrok decoder settings directory for {}: {source}", path.display())]
    CreateParent {
        path: PathBuf,
        #[source]
        source: DocumentError,
    },
    #[error("could not encode Sigrok decoder settings: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("could not write Sigrok decoder settings to {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: DocumentError,
    },
}

struct SigrokDirectoryCatalog {
    documents: NativeDocumentHost,
    settings_path: PathBuf,
    directories: Vec<PathBuf>,
    scanner: Arc<dyn SigrokCatalogScanner>,
    sender: Sender<(u64, Result<SigrokCatalogSnapshot, SigrokCatalogError>)>,
    receiver: Receiver<(u64, Result<SigrokCatalogSnapshot, SigrokCatalogError>)>,
    work_executor: Arc<dyn WorkExecutor>,
    scan_tasks: Vec<Box<dyn WorkTask>>,
    generation: u64,
    scanning: bool,
    discovered: usize,
    diagnostics: Vec<String>,
    settings_error: Option<SigrokCatalogSettingsError>,
    templates: Option<Vec<NodeTemplate>>,
}

impl SigrokDirectoryCatalog {
    fn new(
        documents: NativeDocumentHost,
        settings_path: PathBuf,
        default_directories: Vec<PathBuf>,
        scanner: Arc<dyn SigrokCatalogScanner>,
        work_executor: Arc<dyn WorkExecutor>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (directories, settings_error) = match load_settings(&documents, &settings_path) {
            Ok(Some(settings)) => (settings.directories, None),
            Ok(None) => (default_directories, None),
            Err(error) => (default_directories, Some(error)),
        };
        let mut catalog = Self {
            documents,
            settings_path,
            directories,
            scanner,
            sender,
            receiver,
            work_executor,
            scan_tasks: Vec::new(),
            generation: 0,
            scanning: false,
            discovered: 0,
            diagnostics: Vec::new(),
            settings_error,
            templates: None,
        };
        catalog.start_scan();
        catalog
    }

    fn start_scan(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let directories = self.directories.clone();
        let scanner = Arc::clone(&self.scanner);
        let sender = self.sender.clone();
        self.scanning = true;
        let task = self
            .work_executor
            .submit(Box::new(move || {
                let _ = sender.send((generation, scanner.scan(&directories)));
            }))
            .expect("Sigrok decoder scan task can be scheduled");
        self.scan_tasks.push(task);
    }

    fn poll(&mut self) {
        self.scan_tasks.retain(|task| !task.is_finished());
        while let Ok((generation, result)) = self.receiver.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.scanning = false;
            match result {
                Ok(snapshot) => {
                    self.discovered = snapshot.entries.len();
                    self.diagnostics = snapshot
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.message.clone())
                        .collect();
                    self.templates =
                        Some(logic_analyzer_graph_nodes::sigrok_node_templates(&snapshot));
                }
                Err(error) => self.diagnostics = vec![error.to_string()],
            }
        }
    }

    fn save(&mut self) {
        let settings = SavedSettings {
            directories: self.directories.clone(),
        };
        self.settings_error = save_settings(&self.documents, &self.settings_path, &settings).err();
    }
}

impl NodeCatalogService for SigrokDirectoryCatalog {
    fn snapshot(&mut self) -> NodeCatalogSnapshot {
        self.poll();
        let diagnostics = self
            .settings_error
            .iter()
            .map(ToString::to_string)
            .chain(self.diagnostics.iter().cloned())
            .collect();
        NodeCatalogSnapshot {
            namespace: NAMESPACE.to_owned(),
            title: "External Sigrok Python decoders".to_owned(),
            directories: self
                .directories
                .iter()
                .map(|directory| directory.display().to_string())
                .collect(),
            scanning: self.scanning,
            discovered: self.discovered,
            diagnostics,
        }
    }

    fn add_directory(&mut self) {
        let Some(directory) = self
            .documents
            .choose_directory("Add Sigrok decoder directory", None)
        else {
            return;
        };
        if !self.directories.contains(&directory) {
            self.directories.push(directory);
            self.save();
            self.start_scan();
        }
    }

    fn remove_directory(&mut self, index: usize) {
        if index < self.directories.len() {
            self.directories.remove(index);
            self.save();
            self.start_scan();
        }
    }

    fn rescan(&mut self) {
        self.start_scan();
    }

    fn take_templates(&mut self) -> Option<Vec<NodeTemplate>> {
        self.poll();
        self.templates.take()
    }
}

pub(crate) fn service(
    scanner: Arc<dyn SigrokCatalogScanner>,
    work_executor: Arc<dyn WorkExecutor>,
) -> Box<dyn NodeCatalogService> {
    let documents = NativeDocumentHost::new();
    Box::new(SigrokDirectoryCatalog::new(
        documents,
        documents.configuration_file(logic_analyzer_ui::APPLICATION_ID, "sigrok_decoders.json"),
        default_directories(&documents),
        scanner,
        work_executor,
    ))
}

fn default_directories(documents: &NativeDocumentHost) -> Vec<PathBuf> {
    let mut paths = std::env::var_os("SIGROK_DECODERS_DIR")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    for path in [
        PathBuf::from("/opt/homebrew/share/libsigrokdecode/decoders"),
        PathBuf::from("/usr/local/share/libsigrokdecode/decoders"),
        PathBuf::from("/usr/share/libsigrokdecode/decoders"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../dslogic/libsigrokdecode/decoders"),
    ] {
        if documents.is_directory(&path) && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

fn load_settings(
    documents: &NativeDocumentHost,
    path: &Path,
) -> Result<Option<SavedSettings>, SigrokCatalogSettingsError> {
    let Some(contents) =
        documents
            .read_optional(path)
            .map_err(|source| SigrokCatalogSettingsError::Read {
                path: path.to_owned(),
                source,
            })?
    else {
        return Ok(None);
    };
    serde_json::from_slice(&contents)
        .map(Some)
        .map_err(|source| SigrokCatalogSettingsError::Decode {
            path: path.to_owned(),
            source,
        })
}

fn save_settings(
    documents: &NativeDocumentHost,
    path: &Path,
    settings: &SavedSettings,
) -> Result<(), SigrokCatalogSettingsError> {
    documents
        .create_parent_directories(path)
        .map_err(|source| SigrokCatalogSettingsError::CreateParent {
            path: path.to_owned(),
            source,
        })?;
    let contents =
        serde_json::to_vec_pretty(settings).map_err(SigrokCatalogSettingsError::Encode)?;
    documents
        .write(path, &contents)
        .map_err(|source| SigrokCatalogSettingsError::Write {
            path: path.to_owned(),
            source,
        })
}

#[cfg(test)]
mod sigrok_catalog_settings_tests {
    use std::error::Error;
    use std::path::PathBuf;

    use platform::NativeDocumentHost;

    use super::{SavedSettings, SigrokCatalogSettingsError, load_settings, save_settings};

    #[test]
    fn missing_settings_are_distinct_from_load_failure() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("missing.json");

        assert!(
            load_settings(&NativeDocumentHost::new(), &settings_path)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_settings_retain_the_json_cause() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("sigrok.json");
        std::fs::write(&settings_path, b"not settings").unwrap();

        let error = load_settings(&NativeDocumentHost::new(), &settings_path).unwrap_err();

        assert!(matches!(error, SigrokCatalogSettingsError::Decode { .. }));
        assert!(error.source().unwrap().is::<serde_json::Error>());
    }

    #[test]
    fn settings_round_trip_through_the_document_host() {
        let directory = tempfile::tempdir().unwrap();
        let settings_path = directory.path().join("nested/sigrok.json");
        let settings = SavedSettings {
            directories: vec![PathBuf::from("decoder-a"), PathBuf::from("decoder-b")],
        };
        let documents = NativeDocumentHost::new();

        save_settings(&documents, &settings_path, &settings).unwrap();
        let restored = load_settings(&documents, &settings_path).unwrap().unwrap();

        assert_eq!(restored.directories, settings.directories);
    }

    #[test]
    fn parent_creation_failure_retains_the_document_cause() {
        let directory = tempfile::tempdir().unwrap();
        let occupied_parent = directory.path().join("occupied");
        std::fs::write(&occupied_parent, b"file").unwrap();
        let settings_path = occupied_parent.join("sigrok.json");
        let settings = SavedSettings::default();

        let error =
            save_settings(&NativeDocumentHost::new(), &settings_path, &settings).unwrap_err();

        assert!(matches!(
            error,
            SigrokCatalogSettingsError::CreateParent { .. }
        ));
        assert!(error.source().unwrap().is::<platform::DocumentError>());
    }
}
