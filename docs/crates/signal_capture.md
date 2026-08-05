# `signal_capture` Design

## Responsibility

`signal_capture` owns immutable generic signal capture: packed and edge payloads, source and index
contracts, bounded query and worker protocols, random-access `EdgeQuery`, and finite artifact-backed
waveform indexes. Its crate root is the supported facade; implementation modules are private.

## Dependency boundary

The crate depends only on `signal_artifacts`, `signal_runtime`, and portable serialization,
hashing, error, and channel libraries. It has no dependency on acquisition
sessions, derived-data stores, graph crates, concrete formats or devices, UI, or platform adapters.

Growing live indexes remain with the capture-session owner because they consume mutable session
storage. They reuse an explicit waveform-summary grid contract from this crate, so finite index
algorithms and query semantics remain shared without reversing the dependency.

## Errors and tests

`Error` and `Result` bound capture parsing, indexing, and query failures. Unit tests cover capture
worker protocols, block ownership, finite index construction, bounded sampling, persistence, and
random-access queries. Architecture tests reject session, derived-data, graph, and UI dependencies.
