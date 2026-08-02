use super::{
    ArtifactByteSource, ArtifactKey, ArtifactNamespace, ArtifactRepository, ByteRange,
    MemoryArtifactRepository, PreparedByteSource, ReadArtifact, RepositoryCapabilities,
    RepositoryError, SourceIdentity, read_artifact_region,
};

fn key() -> ArtifactKey {
    ArtifactKey::new(
        ArtifactNamespace::new("derived-words").unwrap(),
        SourceIdentity::from_bytes([4; 32]),
    )
}

#[test]
fn pending_memory_writes_are_invisible_until_published() {
    let repository = MemoryArtifactRepository::new();
    let key = key();
    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.write_at(2, b"abcd").unwrap();

    assert!(repository.open(&key).unwrap().is_none());
    writer.flush().unwrap();
    writer.publish().unwrap();

    let mut reader = repository.open(&key).unwrap().unwrap();
    let mut bytes = [0; 6];
    assert_eq!(reader.read_at(0, &mut bytes).unwrap(), 6);
    assert_eq!(&bytes, b"\0\0abcd");
    assert_eq!(
        reader
            .region(ByteRange::new(2, 4).unwrap())
            .unwrap()
            .unwrap()
            .bytes(),
        b"abcd"
    );
}

#[test]
fn published_memory_artifacts_are_immutable_generations() {
    let repository = MemoryArtifactRepository::new();
    let key = key();
    let mut first = repository.begin_write(key.clone()).unwrap();
    first.write_at(0, b"first").unwrap();
    first.publish().unwrap();
    let mut old_reader = repository.open(&key).unwrap().unwrap();

    let mut replacement = repository.begin_write(key.clone()).unwrap();
    replacement.write_at(0, b"second").unwrap();
    replacement.publish().unwrap();

    let mut old = [0; 5];
    old_reader.read_at(0, &mut old).unwrap();
    let mut current = repository.open(&key).unwrap().unwrap();
    let mut new = [0; 6];
    current.read_at(0, &mut new).unwrap();
    assert_eq!(&old, b"first");
    assert_eq!(&new, b"second");
}

#[test]
fn memory_repository_chunks_ranges_and_enforces_its_budget_atomically() {
    let repository = MemoryArtifactRepository::with_budget_and_chunk_size(8, 3).unwrap();
    let key = key();
    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.write_at(0, b"abcdefgh").unwrap();
    writer.publish().unwrap();
    assert_eq!(repository.used_bytes().unwrap(), 8);

    let mut reader = repository.open(&key).unwrap().unwrap();
    assert!(
        reader
            .region(ByteRange::new(2, 4).unwrap())
            .unwrap()
            .is_none()
    );
    let region = read_artifact_region(&mut *reader, ByteRange::new(2, 4).unwrap()).unwrap();
    assert_eq!(region.bytes(), b"cdef");

    let replacement_key =
        ArtifactKey::new(key.namespace().clone(), SourceIdentity::from_bytes([5; 32]));
    let mut replacement = repository.begin_write(replacement_key.clone()).unwrap();
    replacement.write_at(0, b"x").unwrap();
    assert!(matches!(
        replacement.publish(),
        Err(RepositoryError::QuotaExceeded)
    ));
    assert!(repository.open(&replacement_key).unwrap().is_none());
    assert_eq!(repository.used_bytes().unwrap(), 8);

    repository.remove(&key).unwrap();
    assert_eq!(repository.used_bytes().unwrap(), 0);
}

#[test]
fn memory_repository_lists_and_removes_typed_artifacts() {
    let repository = MemoryArtifactRepository::new();
    let key = key();
    assert_eq!(
        repository.capabilities(),
        RepositoryCapabilities::EPHEMERAL_MEMORY
    );

    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.truncate(3).unwrap();
    writer.publish().unwrap();
    assert_eq!(
        repository.namespaces().unwrap(),
        vec![key.namespace().clone()]
    );
    assert_eq!(repository.entries(key.namespace()).unwrap()[0].length, 3);
    repository.remove(&key).unwrap();
    assert!(repository.entries(key.namespace()).unwrap().is_empty());
    assert!(matches!(
        ArtifactNamespace::new(""),
        Err(RepositoryError::InvalidKey(_))
    ));
}

#[test]
fn repository_artifacts_are_prepared_sources_without_exposing_their_backing() {
    let repository = MemoryArtifactRepository::new();
    let key = key();
    let mut writer = repository.begin_write(key.clone()).unwrap();
    writer.write_at(0, b"prepared").unwrap();
    writer.publish().unwrap();

    let source = ArtifactByteSource::new(std::sync::Arc::new(repository), key);
    let mut reader = source.open_reader().unwrap();
    let mut bytes = [0_u8; 8];
    reader.read_exact_at(0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"prepared");
}

#[test]
fn immutable_regions_fall_back_to_owned_bytes() {
    struct ReadOnlyArtifact {
        key: ArtifactKey,
        bytes: &'static [u8],
        max_read: usize,
    }

    impl ReadArtifact for ReadOnlyArtifact {
        fn key(&self) -> &ArtifactKey {
            &self.key
        }

        fn len(&self) -> Result<u64, RepositoryError> {
            Ok(self.bytes.len() as u64)
        }

        fn read_at(
            &mut self,
            offset: u64,
            destination: &mut [u8],
        ) -> Result<usize, RepositoryError> {
            let start = offset as usize;
            let count = destination
                .len()
                .min(self.bytes.len().saturating_sub(start))
                .min(self.max_read);
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            Ok(count)
        }

        fn region(&self, _range: ByteRange) -> Result<Option<super::ByteRegion>, RepositoryError> {
            Ok(None)
        }
    }

    let mut reader = ReadOnlyArtifact {
        key: key(),
        bytes: b"fallback",
        max_read: 2,
    };
    let region = read_artifact_region(&mut reader, ByteRange::new(2, 4).unwrap()).unwrap();
    assert_eq!(region.bytes(), b"llba");
}
