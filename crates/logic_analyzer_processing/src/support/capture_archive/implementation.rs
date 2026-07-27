use std::fs::File;
use std::io::Read;
use std::path::Path;

use zip::ZipArchive;

use signal_processing::{Error, Result};

pub(crate) trait CaptureArchive: Send {
    fn entry_names(&self) -> Vec<String>;

    fn entry_size(&mut self, name: &str) -> Result<Option<u64>>;

    fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>>;
}

pub(crate) struct ZipCaptureArchive {
    archive: ZipArchive<File>,
}

impl ZipCaptureArchive {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let archive = ZipArchive::new(file).map_err(zip_error)?;
        Ok(Self { archive })
    }
}

impl CaptureArchive for ZipCaptureArchive {
    fn entry_names(&self) -> Vec<String> {
        self.archive.file_names().map(str::to_owned).collect()
    }

    fn entry_size(&mut self, name: &str) -> Result<Option<u64>> {
        match self.archive.by_name(name) {
            Ok(entry) => Ok(Some(entry.size())),
            Err(zip::result::ZipError::FileNotFound) => Ok(None),
            Err(error) => Err(zip_error(error)),
        }
    }

    fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>> {
        let mut entry = match self.archive.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(error) => return Err(zip_error(error)),
        };
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        Ok(Some(data))
    }
}

fn zip_error(error: zip::result::ZipError) -> Error {
    Error::ParseError(format!("capture archive error: {error}"))
}

#[cfg(test)]
mod implementation_tests {
    use std::fs::File;
    use std::io::Write;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn zip_adapter_reads_generated_entries_and_reports_absence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.zip");
        let mut writer = ZipWriter::new(File::create(&path).unwrap());
        writer
            .start_file("metadata", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned fixture").unwrap();
        writer.finish().unwrap();

        let mut archive = ZipCaptureArchive::open(path).unwrap();

        assert_eq!(archive.entry_names(), ["metadata"]);
        assert_eq!(archive.entry_size("metadata").unwrap(), Some(13));
        assert_eq!(
            archive.read_entry("metadata").unwrap(),
            Some(b"owned fixture".to_vec())
        );
        assert_eq!(archive.read_entry("missing").unwrap(), None);
    }
}
