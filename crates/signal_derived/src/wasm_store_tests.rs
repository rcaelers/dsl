use std::sync::Arc;

use platform_artifacts::{ArtifactRepository, MemoryArtifactRepository};

use super::{
    AnnotationQuery, IndexedAnnotationStore, IndexedAnnotationWriter, LiveStoreConfig,
    PersistentStoreConfig, Word,
};

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
