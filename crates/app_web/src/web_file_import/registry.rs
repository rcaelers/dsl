use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use platform_artifacts::{ChunkedByteSource, PreparedByteSource, SourceIdentity};
use signal_capture::CaptureMetadata;

use super::error::BrowserFileRegistryError;
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
    ) -> Result<String, BrowserFileRegistryError> {
        let bytes = bytes.as_ref();
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(BrowserFileRegistryError::FileTooLarge {
                display_name,
                max_mib: MAX_IMPORT_BYTES / (1024 * 1024),
            });
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
    ) -> Result<String, BrowserFileRegistryError> {
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
    ) -> Result<String, BrowserFileRegistryError> {
        let length = chunks.iter().try_fold(0_usize, |total, chunk| {
            total.checked_add(chunk.len()).ok_or_else(|| {
                BrowserFileRegistryError::AddressSpaceOverflow {
                    display_name: display_name.clone(),
                }
            })
        })?;
        if length > MAX_IMPORT_BYTES {
            return Err(BrowserFileRegistryError::FileTooLarge {
                display_name,
                max_mib: MAX_IMPORT_BYTES / (1024 * 1024),
            });
        }
        let source = ChunkedByteSource::new(identity, chunks, IMPORT_CHUNK_BYTES)?;
        let mut state = self.state.lock().unwrap();
        if let Some((reference, _)) = state.files.iter().find(|(_, imported)| {
            imported.display_name == display_name && imported.identity == identity
        }) {
            return Ok(reference.clone());
        }
        if state.resident_bytes > MAX_IMPORT_SESSION_BYTES.saturating_sub(length) {
            return Err(BrowserFileRegistryError::SessionBudgetFull {
                max_mib: MAX_IMPORT_SESSION_BYTES / (1024 * 1024),
            });
        }
        let reference = allocate_reference(&mut state, &display_name)?;
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

    pub(crate) fn allocate_reference(
        &self,
        display_name: &str,
    ) -> Result<String, BrowserFileRegistryError> {
        let mut state = self.state.lock().unwrap();
        allocate_reference(&mut state, display_name)
    }

    pub(crate) fn register_worker_backed(
        &self,
        reference: String,
        display_name: String,
        length: u64,
        identity: SourceIdentity,
        metadata: CaptureMetadata,
    ) -> Result<(), BrowserFileRegistryError> {
        let mut state = self.state.lock().unwrap();
        if state.files.contains_key(&reference) {
            return Err(BrowserFileRegistryError::DuplicateReference { reference });
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

    pub(crate) fn resolve(
        &self,
        reference: &Path,
    ) -> Result<ImportedFile, BrowserFileRegistryError> {
        let reference = reference.to_string_lossy().into_owned();
        self.state
            .lock()
            .unwrap()
            .files
            .get(&reference)
            .cloned()
            .ok_or(BrowserFileRegistryError::UnavailableReference { reference })
    }
}

fn allocate_reference(
    state: &mut RegistryState,
    display_name: &str,
) -> Result<String, BrowserFileRegistryError> {
    state.next_reference = state
        .next_reference
        .checked_add(1)
        .ok_or(BrowserFileRegistryError::ReferenceExhausted)?;
    let safe_name = display_name.replace(['/', '\\'], "_");
    Ok(format!(
        "browser-file://{}/{safe_name}",
        state.next_reference
    ))
}
