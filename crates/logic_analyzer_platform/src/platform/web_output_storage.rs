use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use logic_analyzer_processing::nodes::sinks::{OutputFile, OutputStorage};

thread_local! {
    static OUTPUTS: RefCell<Arc<Mutex<BrowserOutputs>>> = RefCell::new(Arc::new(Mutex::new(BrowserOutputs::default())));
}

#[derive(Default)]
struct BrowserOutputs {
    files: BTreeMap<String, Vec<u8>>,
}

/// A completed browser download emitted by the graph worker.
#[derive(Deserialize, Serialize)]
pub(crate) struct BrowserOutputFile {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

/// Returns the browser's in-memory output destination capability.
pub(crate) fn output_storage() -> Arc<dyn OutputStorage> {
    OUTPUTS.with(|outputs| {
        Arc::new(BrowserOutputStorage {
            outputs: Arc::clone(&outputs.borrow()),
        })
    })
}

/// Drains files closed by browser graph writers for the page's explicit-download queue.
pub(crate) fn take_completed_files() -> Vec<BrowserOutputFile> {
    OUTPUTS.with(|outputs| {
        std::mem::take(&mut outputs.borrow().lock().unwrap().files)
            .into_iter()
            .map(|(name, bytes)| BrowserOutputFile { name, bytes })
            .collect()
    })
}

struct BrowserOutputStorage {
    outputs: Arc<Mutex<BrowserOutputs>>,
}

impl OutputStorage for BrowserOutputStorage {
    fn create_parent_dirs(&self, _path: &Path) -> io::Result<()> {
        // Browser downloads have no directory hierarchy. The file-name mapping
        // below deliberately keeps all writes in the browser-owned download set.
        Ok(())
    }

    fn create(&self, path: &Path) -> io::Result<Box<dyn OutputFile>> {
        let name = browser_file_name(path);
        self.outputs
            .lock()
            .unwrap()
            .files
            .insert(name.clone(), Vec::new());
        Ok(Box::new(BrowserOutputHandle {
            name,
            outputs: Arc::clone(&self.outputs),
        }))
    }

    fn append(&self, path: &Path) -> io::Result<Box<dyn OutputFile>> {
        let name = browser_file_name(path);
        self.outputs
            .lock()
            .unwrap()
            .files
            .entry(name.clone())
            .or_default();
        Ok(Box::new(BrowserOutputHandle {
            name,
            outputs: Arc::clone(&self.outputs),
        }))
    }

    fn exists(&self, path: &Path) -> bool {
        self.outputs
            .lock()
            .unwrap()
            .files
            .contains_key(&browser_file_name(path))
    }
}

struct BrowserOutputHandle {
    name: String,
    outputs: Arc<Mutex<BrowserOutputs>>,
}

impl Write for BrowserOutputHandle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.outputs
            .lock()
            .unwrap()
            .files
            .get_mut(&self.name)
            .expect("browser output handle remains registered")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn browser_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("download")
        .replace(['/', '\\'], "_")
}

#[cfg(test)]
mod web_output_storage_tests {
    use std::io::Write;
    use std::path::Path;

    use super::*;

    #[test]
    fn browser_outputs_collect_writes_by_download_name() {
        let storage = output_storage();
        let mut data = storage.create(Path::new("captures/first.bin")).unwrap();
        data.write_all(&[1, 2]).unwrap();
        drop(data);
        let mut index = storage.append(Path::new("captures/captures.csv")).unwrap();
        index.write_all(b"header\n").unwrap();
        drop(index);

        let outputs = take_completed_files();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "captures.csv");
        assert_eq!(outputs[0].bytes, b"header\n");
        assert_eq!(outputs[1].name, "first.bin");
        assert_eq!(outputs[1].bytes, [1, 2]);
    }
}
