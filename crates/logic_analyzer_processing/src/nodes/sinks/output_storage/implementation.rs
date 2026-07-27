use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

pub(crate) trait OutputFile: Write + Send {}

impl<T> OutputFile for T where T: Write + Send {}

pub(crate) trait OutputStorage: Send + Sync {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()>;

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    fn exists(&self, path: &Path) -> bool;
}

pub(crate) struct NativeOutputStorage;

impl OutputStorage for NativeOutputStorage {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        File::create(path).map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map(|file| Box::new(file) as Box<dyn OutputFile>)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}
