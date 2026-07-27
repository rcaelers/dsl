use std::collections::BTreeMap;
use std::io::{Error, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::implementation::{OutputFile, OutputStorage};

#[derive(Clone, Default)]
pub(crate) struct TestOutputStorage {
    state: Arc<Mutex<TestOutputState>>,
}

#[derive(Default)]
struct TestOutputState {
    files: BTreeMap<PathBuf, Vec<u8>>,
    create_error: Option<ErrorKind>,
    write_error: Option<ErrorKind>,
    flush_error: Option<ErrorKind>,
}

impl TestOutputStorage {
    pub(crate) fn failing_create(error: ErrorKind) -> Self {
        let storage = Self::default();
        storage.state.lock().unwrap().create_error = Some(error);
        storage
    }

    pub(crate) fn failing_write(error: ErrorKind) -> Self {
        let storage = Self::default();
        storage.state.lock().unwrap().write_error = Some(error);
        storage
    }

    pub(crate) fn failing_flush(error: ErrorKind) -> Self {
        let storage = Self::default();
        storage.state.lock().unwrap().flush_error = Some(error);
        storage
    }

    pub(crate) fn contents(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.state.lock().unwrap().files.get(path.as_ref()).cloned()
    }
}

impl OutputStorage for TestOutputStorage {
    fn create_parent_dirs(&self, _path: &Path) -> std::io::Result<()> {
        Ok(())
    }

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        self.open(path, false)
    }

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        self.open(path, true)
    }

    fn exists(&self, path: &Path) -> bool {
        self.state.lock().unwrap().files.contains_key(path)
    }
}

impl TestOutputStorage {
    fn open(&self, path: &Path, append: bool) -> std::io::Result<Box<dyn OutputFile>> {
        let mut state = self.state.lock().unwrap();
        if let Some(kind) = state.create_error {
            return Err(Error::new(kind, "controlled create failure"));
        }
        if !append {
            state.files.insert(path.to_path_buf(), Vec::new());
        } else {
            state.files.entry(path.to_path_buf()).or_default();
        }
        drop(state);
        Ok(Box::new(TestOutputFile {
            path: path.to_path_buf(),
            state: Arc::clone(&self.state),
        }))
    }
}

struct TestOutputFile {
    path: PathBuf,
    state: Arc<Mutex<TestOutputState>>,
}

impl Write for TestOutputFile {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let mut state = self.state.lock().unwrap();
        if let Some(kind) = state.write_error {
            return Err(Error::new(kind, "controlled write failure"));
        }
        state
            .files
            .entry(self.path.clone())
            .or_default()
            .extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let state = self.state.lock().unwrap();
        if let Some(kind) = state.flush_error {
            return Err(Error::new(kind, "controlled flush failure"));
        }
        Ok(())
    }
}
