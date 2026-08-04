use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use signal_artifacts::{ChunkedByteSource, PreparedByteSource, SourceIdentity};
use signal_processing::CaptureMetadata;

use super::worker_source::WorkerCaptureReference;

pub(crate) const IMPORT_CHUNK_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_IMPORT_BYTES: usize = 256 * 1024 * 1024;
const MAX_IMPORT_SESSION_BYTES: usize = 512 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct ImportedFile {
    pub(crate) display_name: String,
    pub(crate) identity: SourceIdentity,
    pub(crate) metadata: Option<CaptureMetadata>,
    pub(crate) source: Option<Arc<dyn PreparedByteSource>>,
    pub(crate) worker_reference: Option<WorkerCaptureReference>,
}

#[derive(Default)]
pub(crate) struct BrowserFileRegistry {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    files: HashMap<String, ImportedFile>,
    resident_bytes: usize,
    next_reference: u64,
}

impl BrowserFileRegistry {
    pub(crate) fn register(
        &self,
        display_name: String,
        bytes: impl AsRef<[u8]>,
    ) -> Result<String, String> {
        let bytes = bytes.as_ref();
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(file_limit_error(&display_name));
        }
        let chunks = bytes
            .chunks(IMPORT_CHUNK_BYTES)
            .map(Arc::<[u8]>::from)
            .collect();
        self.register_chunks(display_name, chunks)
    }

    pub(crate) fn register_chunks(
        &self,
        display_name: String,
        chunks: Vec<Arc<[u8]>>,
    ) -> Result<String, String> {
        let mut hasher = blake3::Hasher::new();
        for chunk in &chunks {
            hasher.update(chunk);
        }
        let identity = SourceIdentity::from_bytes(*hasher.finalize().as_bytes());
        self.register_chunks_with_identity(display_name, chunks, identity)
    }

    pub(crate) fn register_chunks_with_identity(
        &self,
        display_name: String,
        chunks: Vec<Arc<[u8]>>,
        identity: SourceIdentity,
    ) -> Result<String, String> {
        let length = chunks.iter().try_fold(0_usize, |total, chunk| {
            total.checked_add(chunk.len()).ok_or_else(|| {
                format!("'{display_name}' exceeds the browser importer address space")
            })
        })?;
        if length > MAX_IMPORT_BYTES {
            return Err(file_limit_error(&display_name));
        }
        let source = ChunkedByteSource::new(identity, chunks, IMPORT_CHUNK_BYTES)
            .map_err(|error| error.to_string())?;
        let mut state = self.state.lock().unwrap();
        if let Some((reference, _)) = state.files.iter().find(|(_, imported)| {
            imported.display_name == display_name && imported.identity == identity
        }) {
            return Ok(reference.clone());
        }
        if state.resident_bytes > MAX_IMPORT_SESSION_BYTES.saturating_sub(length) {
            return Err(format!(
                "the browser capture import budget is full ({} MiB limit)",
                MAX_IMPORT_SESSION_BYTES / (1024 * 1024)
            ));
        }
        let safe_name = display_name.replace(['/', '\\'], "_");
        state.next_reference = state.next_reference.saturating_add(1);
        let reference = format!("browser-file://{}/{safe_name}", state.next_reference);
        state.resident_bytes += length;
        state.files.insert(
            reference.clone(),
            ImportedFile {
                display_name,
                identity,
                metadata: None,
                source: Some(Arc::new(source)),
                worker_reference: None,
            },
        );
        Ok(reference)
    }

    pub(crate) fn allocate_reference(&self, display_name: &str) -> String {
        let mut state = self.state.lock().unwrap();
        state.next_reference = state.next_reference.saturating_add(1);
        let safe_name = display_name.replace(['/', '\\'], "_");
        format!("browser-file://{}/{safe_name}", state.next_reference)
    }

    pub(crate) fn register_worker_backed(
        &self,
        reference: String,
        display_name: String,
        length: u64,
        identity: SourceIdentity,
        metadata: CaptureMetadata,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.files.contains_key(&reference) {
            return Err(format!(
                "browser capture '{reference}' is already registered"
            ));
        }
        let worker_reference =
            WorkerCaptureReference::new(reference.clone(), display_name.clone(), identity, length);
        state.files.insert(
            reference,
            ImportedFile {
                display_name,
                identity,
                metadata: Some(metadata),
                source: None,
                worker_reference: Some(worker_reference),
            },
        );
        Ok(())
    }

    pub(crate) fn resolve(&self, reference: &Path) -> Result<ImportedFile, String> {
        let reference = reference.to_string_lossy();
        self.state
            .lock()
            .unwrap()
            .files
            .get(reference.as_ref() as &str)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "browser capture '{reference}' is not available in this session; select the file again"
                )
            })
    }
}

pub(crate) fn file_limit_error(display_name: &str) -> String {
    format!(
        "'{display_name}' is too large for the current browser importer ({} MiB limit)",
        MAX_IMPORT_BYTES / (1024 * 1024)
    )
}
