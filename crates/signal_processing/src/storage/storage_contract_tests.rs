use std::sync::Arc;

use super::{
    ByteRange, ImmutableByteRegion, OwnedByteSource, PreparedByteSource, SourceCapabilities,
    SourceIdentity, SourceReadError,
};

fn source() -> OwnedByteSource {
    OwnedByteSource::new(
        SourceIdentity::from_bytes([7; 32]),
        Arc::<[u8]>::from(&b"01234567"[..]),
    )
}

#[test]
fn owned_sources_supply_independent_random_access_readers() {
    let source = source();
    assert_eq!(source.capabilities(), SourceCapabilities::RANDOM_ACCESS);

    let mut first = source.open_reader().unwrap();
    let mut second = source.open_reader().unwrap();
    let mut first_bytes = [0; 3];
    let mut second_bytes = [0; 2];
    first.read_exact_at(2, &mut first_bytes).unwrap();
    second.read_exact_at(6, &mut second_bytes).unwrap();

    assert_eq!(&first_bytes, b"234");
    assert_eq!(&second_bytes, b"67");
}

#[test]
fn immutable_regions_validate_fixed_width_ranges() {
    let source = source();
    let bytes = source.slice(ByteRange::new(1, 4).unwrap()).unwrap();
    assert_eq!(bytes, b"1234");
    assert_eq!(
        source.slice(ByteRange::new(7, 2).unwrap()).unwrap_err(),
        SourceReadError::OutOfBounds {
            offset: 7,
            end: 9,
            source_length: 8,
        }
    );
    assert!(matches!(
        ByteRange::new(u64::MAX, 1),
        Err(SourceReadError::RangeOverflow { .. })
    ));
}

#[test]
fn exact_reads_reject_requests_beyond_the_prepared_source() {
    let mut reader = source().open_reader().unwrap();
    let mut bytes = [0; 4];
    assert_eq!(
        reader.read_exact_at(6, &mut bytes).unwrap_err(),
        SourceReadError::OutOfBounds {
            offset: 6,
            end: 10,
            source_length: 8,
        }
    );
}
