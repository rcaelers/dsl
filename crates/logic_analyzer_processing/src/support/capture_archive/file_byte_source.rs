use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use signal_processing::{
    PreparedByteSource, RandomAccessReader, Result, SourceCapabilities, SourceIdentity,
    SourceReadError,
};

/// Temporary path adapter retained inside the allowlisted file-I/O leaf.
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
