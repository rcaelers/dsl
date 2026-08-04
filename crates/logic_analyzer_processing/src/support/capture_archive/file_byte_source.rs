use std::fs::File;
#[cfg(not(unix))]
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use signal_artifacts::{
    PreparedByteSource, RandomAccessReader, SourceCapabilities, SourceIdentity, SourceReadError,
};
use signal_processing::Result;

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
        #[cfg(unix)]
        {
            self.file
                .read_at(destination, offset)
                .map_err(|error| SourceReadError::Io(error.to_string()))
        }
        #[cfg(not(unix))]
        {
            self.file
                .seek(SeekFrom::Start(offset))
                .and_then(|_| self.file.read(destination))
                .map_err(|error| SourceReadError::Io(error.to_string()))
        }
    }
}

#[cfg(test)]
mod file_byte_source_tests {
    use super::*;

    #[test]
    fn file_reader_supports_cursor_independent_reads() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        let source = FileByteSource::open(&path).unwrap();
        let mut reader = source.open_reader().unwrap();
        let mut later = [0_u8; 3];
        let mut earlier = [0_u8; 2];

        reader.read_exact_at(6, &mut later).unwrap();
        reader.read_exact_at(1, &mut earlier).unwrap();

        assert_eq!(&later, b"678");
        assert_eq!(&earlier, b"12");
    }
}
