use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use zip::ZipArchive;

use signal_processing::{
    Error, PreparedByteSource, RandomAccessReader, Result, SourceCapabilities, SourceIdentity,
    SourceReadError,
};

pub(crate) trait CaptureArchive: Send {
    fn entry_names(&self) -> Vec<String>;

    fn entry_size(&mut self, name: &str) -> Result<Option<u64>>;

    fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>>;
}

/// Temporary native-path adapter retained inside the allowlisted file-I/O leaf.
/// Application composition uses the equivalent host adapter from
/// `logic_analyzer_platform` instead.
pub(crate) struct FileByteSource {
    path: PathBuf,
    identity: SourceIdentity,
}

impl FileByteSource {
    pub(crate) fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        let metadata = std::fs::metadata(&path)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified()
            && let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            hasher.update(&modified.as_nanos().to_le_bytes());
        }
        Ok(Self {
            path,
            identity: SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
        })
    }
}

impl PreparedByteSource for FileByteSource {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> std::result::Result<Box<dyn RandomAccessReader>, SourceReadError> {
        let file =
            File::open(&self.path).map_err(|error| SourceReadError::Io(error.to_string()))?;
        let length = file
            .metadata()
            .map_err(|error| SourceReadError::Io(error.to_string()))?
            .len();
        Ok(Box::new(FileRandomAccessReader { file, length }))
    }
}

struct FileRandomAccessReader {
    file: File,
    length: u64,
}

impl RandomAccessReader for FileRandomAccessReader {
    fn len(&self) -> std::result::Result<u64, SourceReadError> {
        Ok(self.length)
    }

    fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> std::result::Result<usize, SourceReadError> {
        if offset > self.length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: offset,
                source_length: self.length,
            });
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| self.file.read(destination))
            .map_err(|error| SourceReadError::Io(error.to_string()))
    }
}

pub(crate) struct ZipCaptureArchive {
    archive: ZipArchive<Box<dyn ReadSeek + Send>>,
}

impl ZipCaptureArchive {
    pub(crate) fn open_source(source: &dyn PreparedByteSource) -> Result<Self> {
        let reader = source.open_reader().map_err(prepared_source_error)?;
        Self::from_reader(Box::new(RandomAccessCursor::new(reader)?))
    }

    fn from_reader(reader: Box<dyn ReadSeek + Send>) -> Result<Self> {
        let archive = ZipArchive::new(reader).map_err(zip_error)?;
        Ok(Self { archive })
    }
}

trait ReadSeek: Read + Seek {}

impl<T> ReadSeek for T where T: Read + Seek {}

struct RandomAccessCursor {
    reader: Box<dyn RandomAccessReader>,
    position: u64,
    length: u64,
}

impl RandomAccessCursor {
    fn new(reader: Box<dyn RandomAccessReader>) -> Result<Self> {
        let length = reader.len().map_err(prepared_source_error)?;
        Ok(Self {
            reader,
            position: 0,
            length,
        })
    }
}

impl Read for RandomAccessCursor {
    fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
        let count = self
            .reader
            .read_at(self.position, destination)
            .map_err(source_read_io_error)?;
        self.position = self.position.saturating_add(count as u64);
        Ok(count)
    }
}

impl Seek for RandomAccessCursor {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        let position = match position {
            SeekFrom::Start(position) => position as i128,
            SeekFrom::End(offset) => self.length as i128 + offset as i128,
            SeekFrom::Current(offset) => self.position as i128 + offset as i128,
        };
        if !(0..=self.length as i128).contains(&position) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "capture archive seek is outside the prepared source",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
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

fn prepared_source_error(error: SourceReadError) -> Error {
    Error::ParseError(format!("prepared capture source error: {error}"))
}

fn source_read_io_error(error: SourceReadError) -> std::io::Error {
    std::io::Error::other(error)
}

#[cfg(test)]
mod implementation_tests {
    use std::io::{Cursor, Write};
    use std::sync::Arc;

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    use super::*;

    #[test]
    fn zip_adapter_reads_generated_entries_and_reports_absence() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("metadata", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned fixture").unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let source = signal_processing::OwnedByteSource::new(
            SourceIdentity::from_bytes([0x44; 32]),
            Arc::<[u8]>::from(bytes),
        );
        let mut archive = ZipCaptureArchive::open_source(&source).unwrap();

        assert_eq!(archive.entry_names(), ["metadata"]);
        assert_eq!(archive.entry_size("metadata").unwrap(), Some(13));
        assert_eq!(
            archive.read_entry("metadata").unwrap(),
            Some(b"owned fixture".to_vec())
        );
        assert_eq!(archive.read_entry("missing").unwrap(), None);
    }
}
