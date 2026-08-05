use super::{
    ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange, MemoryArtifactRepository,
    RepositoryError, SourceIdentity,
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
