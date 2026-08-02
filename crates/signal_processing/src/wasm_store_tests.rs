use std::sync::Arc;

use crate::{
    AnnotationQuery, ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange,
    IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig, MemoryArtifactRepository,
    PersistentStoreConfig, RepositoryError, SourceIdentity, Word,
};

#[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
fn memory_repository_lifecycle_is_portable_to_wasm() {
    let repository = MemoryArtifactRepository::with_budget_and_chunk_size(8, 3).unwrap();
    let key = ArtifactKey::new(
        ArtifactNamespace::new("wasm-memory-conformance").unwrap(),
        SourceIdentity::from_bytes([0x89; 32]),
    );
    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.write_at(0, b"abcdefgh").unwrap();
    assert!(repository.open(&key).unwrap().is_none());
    writer.publish().unwrap();
    assert_eq!(
        repository
            .open(&key)
            .unwrap()
            .unwrap()
            .region(ByteRange::new(1, 2).unwrap())
            .unwrap()
            .unwrap()
            .bytes(),
        b"bc"
    );

    let overflow = ArtifactKey::new(
        key.namespace().clone(),
        SourceIdentity::from_bytes([0x8a; 32]),
    );
    let mut writer = repository.begin_write(overflow.clone()).unwrap();
    writer.write_at(0, b"x").unwrap();
    assert_eq!(
        writer.publish().unwrap_err(),
        RepositoryError::QuotaExceeded
    );
    assert!(repository.open(&overflow).unwrap().is_none());
}

#[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
fn encoded_store_queries_are_portable_to_the_wasm_memory_repository() {
    let repository: Arc<dyn ArtifactRepository> = Arc::new(MemoryArtifactRepository::new());
    let persistent =
        PersistentStoreConfig::new([0x88; 32]).with_artifact_repository(Arc::clone(&repository));
    let config = LiveStoreConfig {
        hot_tail_publish_words: 1,
        persistence: Some(persistent.clone()),
        ..LiveStoreConfig::default()
    }
    .with_artifact_repository(repository);
    let (mut writer, store) = IndexedAnnotationWriter::create(config).unwrap();
    writer
        .append_batch(&[
            Word::spanning(0x12, 100, 20),
            Word::new(0x34, 200),
            Word::new(0x56, 300),
        ])
        .unwrap();
    assert_eq!(store.exact_window(0, 400, 8).unwrap().annotations.len(), 3);
    writer.finish().unwrap();
    drop(store);

    let reopened = IndexedAnnotationStore::open_persistent(&persistent)
        .unwrap()
        .expect("the encoded memory generation must reopen");
    assert_eq!(
        reopened.exact_window(0, 400, 8).unwrap().annotations.len(),
        3
    );
    assert!(
        reopened
            .presence_window(0, 400, 8)
            .unwrap()
            .iter()
            .map(|bucket| bucket.word_count)
            .sum::<u64>()
            >= 3
    );
    assert_eq!(reopened.nearest_boundary(118, 5).unwrap(), Some(120));
}
