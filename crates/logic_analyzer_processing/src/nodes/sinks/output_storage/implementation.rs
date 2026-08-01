use std::io::Write;
use std::path::Path;

pub trait OutputFile: Write + Send {}

impl<T> OutputFile for T where T: Write + Send {}

pub trait OutputStorage: Send + Sync {
    fn create_parent_dirs(&self, path: &Path) -> std::io::Result<()>;

    fn create(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    fn append(&self, path: &Path) -> std::io::Result<Box<dyn OutputFile>>;

    fn exists(&self, path: &Path) -> bool;
}

pub(crate) struct UnavailableOutputStorage;

impl OutputStorage for UnavailableOutputStorage {
    fn create_parent_dirs(&self, _path: &Path) -> std::io::Result<()> {
        Err(unavailable())
    }

    fn create(&self, _path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        Err(unavailable())
    }

    fn append(&self, _path: &Path) -> std::io::Result<Box<dyn OutputFile>> {
        Err(unavailable())
    }

    fn exists(&self, _path: &Path) -> bool {
        false
    }
}

fn unavailable() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no output destination capability was supplied",
    )
}
