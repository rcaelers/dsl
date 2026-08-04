use std::fs::File;
#[cfg(not(unix))]
use std::io::{Read, Seek, SeekFrom};
#[cfg(unix)]
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use signal_processing::{
    PreparedByteSource, RandomAccessReader, SourceCapabilities, SourceIdentity, SourceReadError,
};

#[derive(Clone, PartialEq, Eq)]
struct NativeFileStamp {
    length: u64,
    modified: Option<SystemTime>,
}

impl NativeFileStamp {
    fn read(path: &Path) -> Result<Self, SourceReadError> {
        let metadata =
            std::fs::metadata(path).map_err(|error| SourceReadError::Io(error.to_string()))?;
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

/// Host-acquired native file exposed through the portable random-access
/// source contract.
pub(crate) struct NativeFileByteSource {
    path: PathBuf,
    identity: SourceIdentity,
    stamp: NativeFileStamp,
}

impl NativeFileByteSource {
    pub(crate) fn acquire(path: impl AsRef<Path>) -> Result<Self, SourceReadError> {
        let path = path.as_ref().to_owned();
        let stamp = NativeFileStamp::read(&path)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&stamp.length.to_le_bytes());
        if let Some(modified) = stamp.modified
            && let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            hasher.update(&modified.as_nanos().to_le_bytes());
        }
        Ok(Self {
            path,
            identity: SourceIdentity::from_bytes(*hasher.finalize().as_bytes()),
            stamp,
        })
    }

    pub(crate) fn open(
        path: impl AsRef<Path>,
        identity: SourceIdentity,
    ) -> Result<Self, SourceReadError> {
        let path = path.as_ref().to_owned();
        let stamp = NativeFileStamp::read(&path)?;
        Ok(Self {
            path,
            identity,
            stamp,
        })
    }
}

impl PreparedByteSource for NativeFileByteSource {
    fn identity(&self) -> SourceIdentity {
        self.identity
    }

    fn capabilities(&self) -> SourceCapabilities {
        SourceCapabilities::RANDOM_ACCESS
    }

    fn open_reader(&self) -> Result<Box<dyn RandomAccessReader>, SourceReadError> {
        if NativeFileStamp::read(&self.path)? != self.stamp {
            return Err(SourceReadError::SourceChanged);
        }
        let file =
            File::open(&self.path).map_err(|error| SourceReadError::Io(error.to_string()))?;
        Ok(Box::new(NativeFileReader {
            file,
            path: self.path.clone(),
            stamp: self.stamp.clone(),
        }))
    }
}

struct NativeFileReader {
    file: File,
    path: PathBuf,
    stamp: NativeFileStamp,
}

impl NativeFileReader {
    fn ensure_unchanged(&self) -> Result<(), SourceReadError> {
        if NativeFileStamp::read(&self.path)? == self.stamp {
            Ok(())
        } else {
            Err(SourceReadError::SourceChanged)
        }
    }
}

impl RandomAccessReader for NativeFileReader {
    fn len(&self) -> Result<u64, SourceReadError> {
        self.ensure_unchanged()?;
        Ok(self.stamp.length)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize, SourceReadError> {
        self.ensure_unchanged()?;
        if offset > self.stamp.length {
            return Err(SourceReadError::OutOfBounds {
                offset,
                end: offset,
                source_length: self.stamp.length,
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
mod native_file_source_tests {
    use super::*;

    #[test]
    fn prepared_file_opens_independent_random_access_readers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.bin");
        std::fs::write(&path, b"0123456789").unwrap();
        let source =
            NativeFileByteSource::open(&path, SourceIdentity::from_bytes([0x31; 32])).unwrap();
        let mut first = source.open_reader().unwrap();
        let mut second = source.open_reader().unwrap();
        let mut first_bytes = [0_u8; 3];
        let mut second_bytes = [0_u8; 2];

        first.read_exact_at(4, &mut first_bytes).unwrap();
        second.read_exact_at(1, &mut second_bytes).unwrap();

        assert_eq!(&first_bytes, b"456");
        assert_eq!(&second_bytes, b"12");
    }

    #[test]
    fn acquired_file_has_a_stable_host_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.bin");
        std::fs::write(&path, b"capture").unwrap();

        let first = NativeFileByteSource::acquire(&path).unwrap();
        let second = NativeFileByteSource::acquire(&path).unwrap();

        assert_eq!(first.identity(), second.identity());
    }

    #[test]
    fn prepared_file_rejects_replacement_after_acquisition() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.bin");
        std::fs::write(&path, b"before").unwrap();
        let source =
            NativeFileByteSource::open(&path, SourceIdentity::from_bytes([0x32; 32])).unwrap();

        std::fs::write(&path, b"different length").unwrap();

        assert!(matches!(
            source.open_reader(),
            Err(SourceReadError::SourceChanged)
        ));
    }
}
