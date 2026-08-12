use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use zip::ZipArchive;

use platform_artifacts::{PreparedByteSource, RandomAccessReader, SourceReadError};
use signal_capture::{Error, Result};

use crate::dsl_file::{ArchiveWorkPhase, ArchiveWorkRecorder, active_archive_work};

pub(crate) trait CaptureArchive: Send {
    fn entry_names(&self) -> Vec<String>;

    fn entry_size(&mut self, name: &str) -> Result<Option<u64>>;

    fn read_entry(&mut self, name: &str) -> Result<Option<Vec<u8>>>;

    fn set_attribution(&mut self, _attribution: Option<ArchiveWorkRecorder>) {}
}

pub(crate) struct ZipCaptureArchive {
    archive: ZipArchive<Box<dyn ReadSeek + Send>>,
    attribution: AttributionSlot,
}

type AttributionSlot = Arc<Mutex<Option<ArchiveWorkRecorder>>>;

impl ZipCaptureArchive {
    pub(crate) fn open_source(source: &dyn PreparedByteSource) -> Result<Self> {
        Self::open_source_with_attribution(source, None)
    }

    pub(crate) fn open_attributed_source(
        source: &dyn PreparedByteSource,
        phase: ArchiveWorkPhase,
    ) -> Result<Self> {
        Self::open_source_with_attribution(source, active_archive_work(source.identity(), phase))
    }

    fn open_source_with_attribution(
        source: &dyn PreparedByteSource,
        attribution: Option<ArchiveWorkRecorder>,
    ) -> Result<Self> {
        let attribution = Arc::new(Mutex::new(attribution));
        let reader = source.open_reader().map_err(prepared_source_error)?;
        Self::from_reader(
            Box::new(RandomAccessCursor::new(reader, Arc::clone(&attribution))?),
            attribution,
        )
    }

    fn from_reader(reader: Box<dyn ReadSeek + Send>, attribution: AttributionSlot) -> Result<Self> {
        let archive = ZipArchive::new(reader).map_err(zip_error)?;
        Ok(Self {
            archive,
            attribution,
        })
    }
}

trait ReadSeek: Read + Seek {}

impl<T> ReadSeek for T where T: Read + Seek {}

struct RandomAccessCursor {
    reader: Box<dyn RandomAccessReader>,
    position: u64,
    length: u64,
    attribution: AttributionSlot,
}

impl RandomAccessCursor {
    fn new(reader: Box<dyn RandomAccessReader>, attribution: AttributionSlot) -> Result<Self> {
        let length = reader.len().map_err(prepared_source_error)?;
        Ok(Self {
            reader,
            position: 0,
            length,
            attribution,
        })
    }
}

impl Read for RandomAccessCursor {
    fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
        let count = self
            .reader
            .read_at(self.position, destination)
            .map_err(source_read_io_error)?;
        if let Some(attribution) = self.attribution.lock().unwrap().as_ref() {
            attribution.record_source_read(self.position, count as u64);
        }
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
            Ok(entry) => {
                if let Some(attribution) = self.attribution.lock().unwrap().as_ref() {
                    attribution
                        .record_entry_open(entry.compression() != zip::CompressionMethod::Stored);
                }
                Ok(Some(entry.size()))
            }
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
        let compressed_bytes = entry.compressed_size();
        let decompressed = entry.compression() != zip::CompressionMethod::Stored;
        if let Some(attribution) = self.attribution.lock().unwrap().as_ref() {
            attribution.record_entry_open(decompressed);
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        if let Some(attribution) = self.attribution.lock().unwrap().as_ref() {
            attribution.record_entry_read(compressed_bytes, data.len() as u64, decompressed);
        }
        Ok(Some(data))
    }

    fn set_attribution(&mut self, attribution: Option<ArchiveWorkRecorder>) {
        *self.attribution.lock().unwrap() = attribution;
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

    use platform_artifacts::SourceIdentity;

    use super::*;

    #[test]
    fn zip_adapter_reads_generated_entries_and_reports_absence() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("metadata", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"owned fixture").unwrap();
        let bytes = writer.finish().unwrap().into_inner();

        let source = platform_artifacts::OwnedByteSource::new(
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
