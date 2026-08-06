use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use signal_sinks::{OutputFile, OutputOrigin, OutputStorage};

thread_local! {
    static OUTPUTS: RefCell<Arc<Mutex<BrowserOutputs>>> = RefCell::new(Arc::new(Mutex::new(BrowserOutputs::default())));
}

#[derive(Default)]
struct BrowserOutputs {
    files: BTreeMap<String, BrowserStoredOutput>,
}

struct BrowserStoredOutput {
    bytes: Vec<u8>,
    producer_node: String,
    producer_socket: String,
}

/// A completed browser download emitted by the graph worker.
#[derive(Deserialize, Serialize)]
pub(crate) struct BrowserOutputFile {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) producer_node: String,
    pub(crate) producer_socket: String,
}

/// Returns the browser's in-memory output destination capability.
pub(crate) fn output_storage() -> Arc<dyn OutputStorage> {
    OUTPUTS.with(|outputs| {
        Arc::new(BrowserOutputStorage {
            outputs: Arc::clone(&outputs.borrow()),
        })
    })
}

/// Discards files from an incomplete earlier execution before a new run starts.
pub(crate) fn begin_output_run() {
    OUTPUTS.with(|outputs| outputs.borrow().lock().unwrap().files.clear());
}

/// Drains files closed by browser graph writers for the page's explicit-download queue.
pub(crate) fn take_completed_files() -> Vec<BrowserOutputFile> {
    OUTPUTS.with(|outputs| {
        std::mem::take(&mut outputs.borrow().lock().unwrap().files)
            .into_iter()
            .map(|(name, output)| BrowserOutputFile {
                name,
                bytes: output.bytes,
                producer_node: output.producer_node,
                producer_socket: output.producer_socket,
            })
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
        self.create_for(path, &OutputOrigin::new("Unknown node", "Unknown socket"))
    }

    fn create_for(&self, path: &Path, origin: &OutputOrigin) -> io::Result<Box<dyn OutputFile>> {
        let name = browser_file_name(path);
        self.outputs.lock().unwrap().files.insert(
            name.clone(),
            BrowserStoredOutput {
                bytes: Vec::new(),
                producer_node: origin.node.to_owned(),
                producer_socket: origin.socket.to_owned(),
            },
        );
        Ok(Box::new(BrowserOutputHandle {
            name,
            outputs: Arc::clone(&self.outputs),
        }))
    }

    fn append(&self, path: &Path) -> io::Result<Box<dyn OutputFile>> {
        self.append_for(path, &OutputOrigin::new("Unknown node", "Unknown socket"))
    }

    fn append_for(&self, path: &Path, origin: &OutputOrigin) -> io::Result<Box<dyn OutputFile>> {
        let name = browser_file_name(path);
        self.outputs
            .lock()
            .unwrap()
            .files
            .entry(name.clone())
            .and_modify(|output| {
                output.producer_node = origin.node.to_owned();
                output.producer_socket = origin.socket.to_owned();
            })
            .or_insert_with(|| BrowserStoredOutput {
                bytes: Vec::new(),
                producer_node: origin.node.to_owned(),
                producer_socket: origin.socket.to_owned(),
            });
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
            .bytes
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
        begin_output_run();
        let mut data = storage
            .create_for(
                Path::new("captures/first.bin"),
                &OutputOrigin::new("Parallel Decoder", "Words"),
            )
            .unwrap();
        data.write_all(&[1, 2]).unwrap();
        drop(data);
        let mut index = storage
            .append_for(
                Path::new("captures/captures.csv"),
                &OutputOrigin::new("Parallel Decoder", "Words"),
            )
            .unwrap();
        index.write_all(b"header\n").unwrap();
        drop(index);

        let outputs = take_completed_files();
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name, "captures.csv");
        assert_eq!(outputs[0].bytes, b"header\n");
        assert_eq!(outputs[0].producer_node, "Parallel Decoder");
        assert_eq!(outputs[0].producer_socket, "Words");
        assert_eq!(outputs[1].name, "first.bin");
        assert_eq!(outputs[1].bytes, [1, 2]);
        assert_eq!(outputs[1].producer_node, "Parallel Decoder");
        assert_eq!(outputs[1].producer_socket, "Words");
    }
}
