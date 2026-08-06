use std::collections::BTreeSet;
use std::sync::Arc;

use platform_artifacts::{
    ArtifactByteSource, ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange,
    PreparedByteSource, RepositoryCapabilities, SourceCapabilities, SourceIdentity, UnixTimeSource,
};
use signal_capture_session::{
    CaptureChannelId, CaptureChunk, CaptureChunkWriter, CaptureCursorItem, CaptureSessionId,
    CaptureStore, CaptureStoreConfig, CaptureStoreCursor, CaptureStoreDescriptor, FinalizedCapture,
};
use signal_derived::{
    Annotation, AnnotationQuery, BlockCodecConfig, IndexedAnnotationStore, IndexedAnnotationWriter,
    LiveStoreConfig, PersistentStoreConfig, Word, WordPresenceBucket,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ArtifactSnapshot {
    namespace: String,
    identity: [u8; 32],
    bytes: Vec<u8>,
}

/// Complete deterministic artifact generation produced by a conformance fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryConformanceSnapshot {
    artifacts: Vec<ArtifactSnapshot>,
}

/// Query results and encoded generation produced by the derived-store fixture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DerivedStoreConformanceSnapshot {
    live_values: Vec<u64>,
    exact: Vec<Annotation>,
    presence: Vec<WordPresenceBucket>,
    nearest_boundary: Option<u64>,
    repository: RepositoryConformanceSnapshot,
}

struct FixedTimeSource(u64);

impl UnixTimeSource for FixedTimeSource {
    fn now_unix_ns(&self) -> u64 {
        self.0
    }
}

/// Verifies the common artifact lifecycle and immutable-region contract.
///
/// # Parameters
/// - `repository`: Input consumed by this operation.
/// - `expected_capabilities`: Input consumed by this operation.
pub fn repository_conformance(
    repository: Arc<dyn ArtifactRepository>,
    expected_capabilities: RepositoryCapabilities,
) {
    assert_eq!(repository.capabilities(), expected_capabilities);
    let namespace = ArtifactNamespace::new("data-plane-conformance").unwrap();
    let key = ArtifactKey::new(namespace.clone(), SourceIdentity::from_bytes([0x5a; 32]));

    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.write_at(2, b"cde").unwrap();
    writer.write_at(0, b"ab").unwrap();
    assert!(repository.open(&key).unwrap().is_none());
    writer.flush().unwrap();
    writer.publish().unwrap();

    let source = ArtifactByteSource::new(Arc::clone(&repository), key.clone());
    assert_eq!(source.capabilities(), SourceCapabilities::RANDOM_ACCESS);
    let mut source_reader = source.open_reader().unwrap();
    let mut bytes = [0_u8; 5];
    source_reader.read_exact_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"abcde");

    let mut reader = repository.open(&key).unwrap().unwrap();
    let mut tail = [0_u8; 4];
    assert_eq!(reader.read_at(3, &mut tail).unwrap(), 2);
    assert_eq!(&tail[..2], b"de");
    assert_eq!(
        reader
            .region(ByteRange::new(1, 3).unwrap())
            .unwrap()
            .expect("immutable repositories expose a resident region")
            .bytes(),
        b"bcd"
    );
    assert_eq!(repository.entries(&namespace).unwrap().len(), 1);
    assert_eq!(repository.namespaces().unwrap(), vec![namespace.clone()]);

    let mut previous_generation = repository.open(&key).unwrap().unwrap();
    let mut replacement = repository.begin_write(key.clone()).unwrap();
    replacement.write_at(0, b"vwxyz").unwrap();
    replacement.publish().unwrap();
    let mut previous = [0_u8; 5];
    previous_generation.read_at(0, &mut previous).unwrap();
    assert_eq!(&previous, b"abcde");

    let mut unpublished = repository.begin_write(key.clone()).unwrap();
    unpublished.write_at(0, b"incomplete").unwrap();
    drop(unpublished);
    let mut current = repository.open(&key).unwrap().unwrap();
    let mut preserved = [0_u8; 5];
    current.read_at(0, &mut preserved).unwrap();
    assert_eq!(&preserved, b"vwxyz");

    repository.remove(&key).unwrap();
    assert!(repository.entries(&namespace).unwrap().is_empty());
    assert!(repository.namespaces().unwrap().is_empty());
}

/// Exercises growing-prefix visibility and finalized replay through a repository.
pub fn capture_store_conformance(
    repository: Arc<dyn ArtifactRepository>,
) -> RepositoryConformanceSnapshot {
    let descriptor = CaptureStoreDescriptor::new(
        CaptureSessionId::new(0xace),
        [
            CaptureChannelId::new("clock"),
            CaptureChannelId::new("data"),
        ],
    )
    .unwrap();
    let clock: Arc<dyn UnixTimeSource> = Arc::new(FixedTimeSource(1_234_567_890));
    let (store, mut writer) = CaptureStore::create(
        CaptureStoreConfig::new(Arc::clone(&repository), descriptor.clone())
            .with_time_source(clock),
    )
    .unwrap();
    writer
        .append(
            CaptureChunk::packed_lsb_first(
                descriptor.session_id(),
                0,
                0,
                4,
                descriptor.channel_table(),
                [0b0110_1001],
                0,
            )
            .unwrap(),
        )
        .unwrap();

    let mut live_cursor = store.open_cursor().unwrap();
    let CaptureCursorItem::Chunk(live_chunk) = live_cursor.next().unwrap() else {
        panic!("the committed prefix must be visible before capture completion");
    };
    assert_eq!(live_chunk.packed_level(0, 0), Some(true));
    assert_eq!(live_cursor.next().unwrap(), CaptureCursorItem::Pending);

    writer.finish().unwrap();
    let finalized = store.finalize().unwrap();
    assert_eq!(finalized.generation(), 2);
    let reopened =
        FinalizedCapture::open(Arc::clone(&repository), descriptor.session_id()).unwrap();
    let mut cursor = reopened.open_cursor().unwrap();
    let CaptureCursorItem::Chunk(chunk) = cursor.next().unwrap() else {
        panic!("the finalized capture must replay its committed chunk");
    };
    assert_eq!(chunk.packed_level(0, 0), Some(true));
    assert_eq!(chunk.packed_level(0, 1), Some(false));
    assert_eq!(cursor.next().unwrap(), CaptureCursorItem::End);

    let snapshot = repository_snapshot(repository.as_ref());
    verify_capture_corruption(repository);
    snapshot
}

/// Exercises deterministic encoding plus live and reopened query behavior.
///
/// # Parameters
/// - `repository`: Input consumed by this operation.
pub fn derived_store_conformance(
    repository: Arc<dyn ArtifactRepository>,
) -> DerivedStoreConformanceSnapshot {
    let clock: Arc<dyn UnixTimeSource> = Arc::new(FixedTimeSource(9_876_543_210));
    let persistent = PersistentStoreConfig::new([0x71; 32])
        .with_artifact_repository(Arc::clone(&repository))
        .with_time_source(clock);
    let config = LiveStoreConfig {
        block: BlockCodecConfig {
            max_words: 2,
            restart_interval: 1,
            ..BlockCodecConfig::default()
        },
        hot_tail_publish_words: 1,
        persistence: Some(persistent.clone()),
        ..LiveStoreConfig::default()
    }
    .with_artifact_repository(Arc::clone(&repository));
    let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
    writer
        .append_batch(&[Word::spanning(0x12, 100, 20), Word::new(0x34, 200)])
        .unwrap();
    let live_values = store
        .exact_window(0, 500, 16)
        .unwrap()
        .annotations
        .into_iter()
        .map(|annotation| annotation.value)
        .collect::<Vec<_>>();
    assert_eq!(live_values, vec![0x12, 0x34]);

    writer
        .append_batch(&[
            Word::bytes_with_tag(0x56, [0xaa, 0x55], 300, 10),
            Word::text("ready", 450, 5),
        ])
        .unwrap();
    writer.finish().unwrap();
    drop(store);

    let reopened = IndexedAnnotationStore::open_persistent(&persistent)
        .unwrap()
        .expect("the persistent generation must reopen");
    let exact = reopened.exact_window(0, 600, 16).unwrap().annotations;
    let presence = reopened.presence_window(0, 600, 16).unwrap();
    let nearest_boundary = reopened.nearest_boundary(118, 5).unwrap();
    assert_eq!(exact.len(), 4);
    assert!(presence.iter().map(|bucket| bucket.word_count).sum::<u64>() >= 4);
    assert_eq!(nearest_boundary, Some(120));

    let snapshot = DerivedStoreConformanceSnapshot {
        live_values,
        exact,
        presence,
        nearest_boundary,
        repository: repository_snapshot(repository.as_ref()),
    };

    let cancelled = PersistentStoreConfig::new([0x72; 32])
        .with_artifact_repository(Arc::clone(&repository))
        .with_time_source(Arc::new(FixedTimeSource(9_876_543_210)));
    let cancel_config = LiveStoreConfig {
        persistence: Some(cancelled.clone()),
        ..LiveStoreConfig::default()
    }
    .with_artifact_repository(repository);
    let (mut writer, store) = IndexedAnnotationWriter::create(cancel_config).unwrap();
    writer.append(Word::new(0xff, 1_000)).unwrap();
    writer.cancel().unwrap();
    drop(writer);
    drop(store);
    assert!(
        IndexedAnnotationStore::open_persistent(&cancelled)
            .unwrap()
            .is_none()
    );
    verify_derived_corruption(cancelled.artifact_repository);

    snapshot
}

fn verify_capture_corruption(repository: Arc<dyn ArtifactRepository>) {
    let chunk_namespace = ArtifactNamespace::new("capture-chunk-v1").unwrap();
    let previous = repository
        .entries(&chunk_namespace)
        .unwrap()
        .into_iter()
        .map(|metadata| metadata.key.identity())
        .collect::<BTreeSet<_>>();
    let descriptor = CaptureStoreDescriptor::new(
        CaptureSessionId::new(0xacf),
        [CaptureChannelId::new("data")],
    )
    .unwrap();
    let (store, mut writer) = CaptureStore::create(
        CaptureStoreConfig::new(Arc::clone(&repository), descriptor.clone())
            .with_time_source(Arc::new(FixedTimeSource(1_234_567_890))),
    )
    .unwrap();
    writer
        .append(
            CaptureChunk::packed_lsb_first(
                descriptor.session_id(),
                0,
                0,
                4,
                descriptor.channel_table(),
                [0b0000_1010],
                0,
            )
            .unwrap(),
        )
        .unwrap();
    writer.finish().unwrap();
    store.finalize().unwrap();
    let chunk = repository
        .entries(&chunk_namespace)
        .unwrap()
        .into_iter()
        .find(|metadata| !previous.contains(&metadata.key.identity()))
        .expect("the corruption fixture publishes one new capture chunk");
    let mut replacement = repository.begin_write(chunk.key).unwrap();
    replacement.write_at(0, b"corrupt").unwrap();
    replacement.publish().unwrap();

    let reopened = FinalizedCapture::open(repository, descriptor.session_id()).unwrap();
    assert!(reopened.open_cursor().unwrap().next().is_err());
}

fn verify_derived_corruption(repository: Arc<dyn ArtifactRepository>) {
    let cache_key = [0x73; 32];
    let persistent = PersistentStoreConfig::new(cache_key)
        .with_artifact_repository(Arc::clone(&repository))
        .with_time_source(Arc::new(FixedTimeSource(9_876_543_210)));
    let config = LiveStoreConfig {
        persistence: Some(persistent.clone()),
        ..LiveStoreConfig::default()
    }
    .with_artifact_repository(Arc::clone(&repository));
    let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
    writer.append(Word::new(0xab, 2_000)).unwrap();
    writer.finish().unwrap();
    drop(store);

    let namespace = ArtifactNamespace::new("derived-word-index-v1").unwrap();
    let index = repository
        .entries(&namespace)
        .unwrap()
        .into_iter()
        .find(|metadata| metadata.key.identity().as_bytes() == &cache_key)
        .expect("the corruption fixture publishes its persistent index");
    let mut replacement = repository.begin_write(index.key).unwrap();
    replacement.write_at(0, b"corrupt").unwrap();
    replacement.publish().unwrap();

    assert!(IndexedAnnotationStore::open_persistent(&persistent).is_err());
}

fn repository_snapshot(repository: &dyn ArtifactRepository) -> RepositoryConformanceSnapshot {
    let mut artifacts = Vec::new();
    for namespace in repository.namespaces().unwrap() {
        for metadata in repository.entries(&namespace).unwrap() {
            let mut reader = repository.open(&metadata.key).unwrap().unwrap();
            let length = usize::try_from(metadata.length).unwrap();
            let mut bytes = vec![0_u8; length];
            let mut copied = 0;
            while copied < bytes.len() {
                let count = reader
                    .read_at(u64::try_from(copied).unwrap(), &mut bytes[copied..])
                    .unwrap();
                assert!(count > 0, "a listed artifact must not be truncated");
                copied += count;
            }
            artifacts.push(ArtifactSnapshot {
                namespace: namespace.as_str().to_owned(),
                identity: *metadata.key.identity().as_bytes(),
                bytes,
            });
        }
    }
    artifacts.sort_by(|left, right| {
        (&left.namespace, left.identity).cmp(&(&right.namespace, right.identity))
    });
    RepositoryConformanceSnapshot { artifacts }
}

#[cfg(test)]
mod repository_tests {
    use platform_artifacts::{MemoryArtifactRepository, RepositoryCapabilities, RepositoryError};

    use super::*;

    #[test]
    fn memory_repository_runs_the_reusable_data_plane_conformance_suite() {
        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        repository_conformance(
            Arc::clone(&repository),
            RepositoryCapabilities::EPHEMERAL_MEMORY,
        );
        assert!(
            !capture_store_conformance(Arc::clone(&repository))
                .artifacts
                .is_empty()
        );

        let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
        assert!(
            !derived_store_conformance(repository)
                .repository
                .artifacts
                .is_empty()
        );
    }

    #[test]
    fn memory_repository_reports_quota_exhaustion_without_partial_publication() {
        let repository = MemoryArtifactRepository::with_budget(4);
        let key = ArtifactKey::new(
            ArtifactNamespace::new("quota-conformance").unwrap(),
            SourceIdentity::from_bytes([0x44; 32]),
        );
        let mut writer = repository.begin_write(key.clone()).unwrap();

        assert_eq!(
            writer.write_at(0, b"12345").unwrap_err(),
            RepositoryError::QuotaExceeded
        );
        drop(writer);
        assert!(repository.open(&key).unwrap().is_none());
    }
}
