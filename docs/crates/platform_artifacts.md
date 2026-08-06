# `platform_artifacts` Design

## Responsibility

`platform_artifacts` owns platform-neutral immutable byte regions, stable source and artifact
identities, artifact repository contracts, the portable in-memory repository, prepared byte-source
contracts, repository replication events, and shared persistence time and checksum primitives.

## Public facade

The crate root is the only supported facade. It selectively exposes byte ranges and regions,
prepared random-access sources, artifact keys and metadata, repository reader and writer contracts,
the in-memory repository, repository-backed byte sources, replication contracts, clocks, and
checksums. Consumers import these symbols from `platform_artifacts`; higher-level owners do not
re-export them.

## Dependencies and boundary

The crate depends only on serialization, timing, and error support. It has no dependency on
capture or derived-data formats, graph crates, UI crates, platform adapters,
or compilation targets.

Native mmap/filesystem repositories and browser persistence repositories implement these contracts
in `platform`. Capture and derived-data owners choose their namespaces and encodings;
`platform_artifacts` does not assign application cache policy or interpret stored bytes.

## Invariants and errors

Published artifacts are immutable generations. Pending writes remain invisible until publication,
and dropping a writer never publishes incomplete data. Byte ranges are validated before access.
`RepositoryError` and `SourceReadError` are the crate boundary errors and contain no host-specific
error types.

## Test boundary

Contract tests cover repository lifecycle, range validation, memory budgeting, immutable regions,
replication ordering, and prepared-source portability. Native and browser repository conformance is
tested by their platform owners against the same public contracts.
