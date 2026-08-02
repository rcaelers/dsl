use signal_processing::{CaptureDataSource, SourceIdentity};

pub(crate) fn capture_cache_identity<S>(source_identity: SourceIdentity, source: &S) -> [u8; 32]
where
    S: CaptureDataSource,
{
    let metadata = source.metadata();
    let mut hasher = blake3::Hasher::new();
    hasher.update(source_identity.as_bytes());
    hasher.update(&source.fingerprint().revision.to_le_bytes());
    hasher.update(&metadata.samplerate_hz.to_bits().to_le_bytes());
    hasher.update(&metadata.total_samples.to_le_bytes());
    hasher.update(&(metadata.total_probes as u64).to_le_bytes());
    for name in &metadata.probe_names {
        hasher.update(name.as_bytes());
    }
    *hasher.finalize().as_bytes()
}
