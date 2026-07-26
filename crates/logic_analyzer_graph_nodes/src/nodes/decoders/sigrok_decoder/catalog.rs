use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use serde::{Deserialize, Serialize};

use logic_analyzer_graph_api::node::{DirectoryNodeCatalog, NodeCatalogStatus};
use logic_analyzer_processing::nodes::decoders::sigrok_decoder::{
    SigrokCatalogSnapshot, SigrokDecoderCatalog,
};
use node_graph::NodeTemplate;

use super::definition::SigrokDecoderState;

const NAMESPACE: &str = "logic_conduit.sigrok_python";

#[derive(Default, Deserialize, Serialize)]
struct SavedSettings {
    directories: Vec<PathBuf>,
}

pub(crate) struct SigrokDirectoryCatalog {
    settings_path: PathBuf,
    directories: Vec<PathBuf>,
    sender: Sender<(u64, SigrokCatalogSnapshot)>,
    receiver: Receiver<(u64, SigrokCatalogSnapshot)>,
    generation: u64,
    scanning: bool,
    discovered: usize,
    diagnostics: Vec<String>,
    templates: Option<Vec<NodeTemplate>>,
}

impl SigrokDirectoryCatalog {
    pub(crate) fn new(settings_path: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let directories = load_settings(&settings_path)
            .map(|settings| settings.directories)
            .unwrap_or_else(super::definition::default_decoder_search_paths);
        let mut catalog = Self {
            settings_path,
            directories,
            sender,
            receiver,
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
        let sender = self.sender.clone();
        self.scanning = true;
        std::thread::Builder::new()
            .name("sigrok-decoder-scan".to_owned())
            .spawn(move || {
                let snapshot = SigrokDecoderCatalog::default().refresh(&directories);
                let _ = sender.send((generation, (*snapshot).clone()));
            })
            .expect("Sigrok decoder scan thread can be started");
    }

    fn poll(&mut self) {
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
            self.templates = Some(
                snapshot
                    .entries
                    .iter()
                    .map(|entry| {
                        let descriptor = &entry.descriptor;
                        let tag = descriptor.tags.first().map_or("Other", String::as_str);
                        NodeTemplate {
                            name: format!("{} ({})", descriptor.name, descriptor.id),
                            category: format!("External Sigrok::{tag}"),
                            base_type: "Sigrok Decoder".to_owned(),
                            title: format!("{} · Sigrok", descriptor.name),
                            state: serde_json::to_value(SigrokDecoderState::from_descriptor(
                                entry.decoder_root.clone(),
                                descriptor,
                            ))
                            .expect("Sigrok decoder template state is serializable"),
                        }
                    })
                    .collect(),
            );
        }
    }

    fn save(&mut self) {
        let settings = SavedSettings {
            directories: self.directories.clone(),
        };
        let result = self
            .settings_path
            .parent()
            .ok_or_else(|| "settings path has no parent".to_owned())
            .and_then(|parent| std::fs::create_dir_all(parent).map_err(|error| error.to_string()))
            .and_then(|()| serde_json::to_vec_pretty(&settings).map_err(|error| error.to_string()))
            .and_then(|bytes| {
                std::fs::write(&self.settings_path, bytes).map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.diagnostics.push(format!(
                "Could not save Sigrok decoder settings to {}: {error}",
                self.settings_path.display()
            ));
        }
    }
}

impl DirectoryNodeCatalog for SigrokDirectoryCatalog {
    fn namespace(&self) -> &str {
        NAMESPACE
    }

    fn title(&self) -> &str {
        "External Sigrok Python decoders"
    }

    fn directories(&self) -> Vec<PathBuf> {
        self.directories.clone()
    }

    fn set_directories(&mut self, directories: Vec<PathBuf>) {
        self.directories = directories;
        self.save();
        self.start_scan();
    }

    fn rescan(&mut self) {
        self.start_scan();
    }

    fn status(&mut self) -> NodeCatalogStatus {
        self.poll();
        NodeCatalogStatus {
            scanning: self.scanning,
            discovered: self.discovered,
            diagnostics: self.diagnostics.clone(),
        }
    }

    fn take_templates(&mut self) -> Option<Vec<NodeTemplate>> {
        self.poll();
        self.templates.take()
    }
}

fn load_settings(path: &PathBuf) -> Option<SavedSettings> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod catalog_tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn background_scan_publishes_clearly_external_templates() {
        let directory = tempfile::tempdir().unwrap();
        let decoder_root = directory.path().join("decoders");
        write_fixture(&decoder_root);
        let settings_path = directory.path().join("sigrok_decoders.json");
        std::fs::write(
            &settings_path,
            serde_json::to_vec(&SavedSettings {
                directories: vec![decoder_root],
            })
            .unwrap(),
        )
        .unwrap();
        let mut catalog = SigrokDirectoryCatalog::new(settings_path);
        let deadline = Instant::now() + Duration::from_secs(5);

        let templates = loop {
            if let Some(templates) = catalog.take_templates() {
                break templates;
            }
            assert!(
                Instant::now() < deadline,
                "background catalog scan timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "Foreign fixture (foreign_fixture)");
        assert_eq!(templates[0].category, "External Sigrok::Test instruments");
        assert_eq!(templates[0].base_type, "Sigrok Decoder");
    }

    fn write_fixture(root: &std::path::Path) {
        let package = root.join("foreign_fixture");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("__init__.py"), "from .pd import Decoder\n").unwrap();
        std::fs::write(
            package.join("pd.py"),
            r#"import sigrokdecode as srd

class Decoder(srd.Decoder):
    api_version = 3
    id = 'foreign_fixture'
    name = 'Foreign fixture'
    longname = 'Foreign fixture'
    desc = 'Fixture decoder.'
    license = 'mit'
    inputs = ['logic']
    outputs = []
    tags = ['Test instruments']
    channels = ({'id': 'data', 'name': 'Data', 'desc': 'Data'},)
    optional_channels = ()
    options = ()
    annotations = ()
    annotation_rows = ()
    binary = ()

    def metadata(self, key, value):
        self.samplerate = value

    def start(self):
        pass
"#,
        )
        .unwrap();
    }
}
