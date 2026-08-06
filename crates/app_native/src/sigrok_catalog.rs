//! Native composition of Sigrok discovery with the UI node-catalog port.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogScanner, SigrokCatalogSnapshot,
};
use logic_analyzer_ui::{NodeCatalogService, NodeCatalogSnapshot};
use node_graph::NodeTemplate;
use platform::NativeDocumentHost;
use platform_runtime::{WorkExecutor, WorkTask};

const NAMESPACE: &str = "logic_conduit.sigrok_python";

#[derive(Default, Deserialize, Serialize)]
struct SavedSettings {
    directories: Vec<PathBuf>,
}

struct SigrokDirectoryCatalog {
    documents: NativeDocumentHost,
    settings_path: PathBuf,
    directories: Vec<PathBuf>,
    scanner: Arc<dyn SigrokCatalogScanner>,
    sender: Sender<(u64, SigrokCatalogSnapshot)>,
    receiver: Receiver<(u64, SigrokCatalogSnapshot)>,
    work_executor: Arc<dyn WorkExecutor>,
    scan_tasks: Vec<Box<dyn WorkTask>>,
    generation: u64,
    scanning: bool,
    discovered: usize,
    diagnostics: Vec<String>,
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
        let directories = load_settings(&documents, &settings_path)
            .map(|settings| settings.directories)
            .unwrap_or(default_directories);
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
        while let Ok((generation, snapshot)) = self.receiver.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.scanning = false;
            self.discovered = snapshot.entries.len();
            self.diagnostics = snapshot
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect();
            self.templates = Some(logic_analyzer_graph_nodes::sigrok_node_templates(&snapshot));
        }
    }

    fn save(&mut self) {
        let settings = SavedSettings {
            directories: self.directories.clone(),
        };
        let result = self
            .documents
            .create_parent_directories(&self.settings_path)
            .and_then(|()| serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string()))
            .and_then(|bytes| self.documents.write(&self.settings_path, &bytes));
        if let Err(error) = result {
            self.diagnostics.push(format!(
                "Could not save Sigrok decoder settings to {}: {error}",
                self.settings_path.display()
            ));
        }
    }
}

impl NodeCatalogService for SigrokDirectoryCatalog {
    fn snapshot(&mut self) -> NodeCatalogSnapshot {
        self.poll();
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
            diagnostics: self.diagnostics.clone(),
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

fn load_settings(documents: &NativeDocumentHost, path: &Path) -> Option<SavedSettings> {
    serde_json::from_slice(&documents.read_optional(path).ok()??).ok()
}
