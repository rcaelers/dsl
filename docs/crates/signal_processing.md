# `signal_processing` Design

## Responsibility

`signal_processing` owns portable generic signal contracts and the independently changing capture,
derived-data, capture-session, and stream-execution domains that have not been extracted into a
lower crate. It consumes immutable byte and artifact contracts directly from `signal_artifacts`.
It owns no concrete device, capture format, protocol decoder, graph node, UI, host adapter, or
target selection.

## Stream-execution owner

The private `runtime` facade owns typed ports and channels, `ProcessNode`, graph and pipeline
construction, threaded and cooperative schedulers and managers, watchdogs, runtime errors, and
generic worker execution. Its leaves collaborate through direct sibling paths. Other internal
owners consume the `runtime` facade, while supported cross-crate contracts are selectively exposed
by the crate root for existing consumers.

The runtime owner may depend on the generic `Sample`, `SampleBlock`, `SampleKind`, event, and
`EdgeQuery` contracts needed by current port negotiation. It does not depend on capture stores,
acquisition policy, derived stores, payload collection, finite waveform indexes, or artifact
repositories. Architecture tests enforce that dependency allowlist and the single directory-backed
owner facade.

## Other owners

The public `capture` and `waveform_index` facades own generic capture preparation, query, indexing,
and finite waveform behavior. `derived_word_store` and the private derived-data facades own payload
collection, sampling, indexing, and encoded derived storage. `live_capture`, `live_capture_store`,
and `logic_analyzer` own driver-neutral acquisition, session storage, policy, trigger, and capture
source contracts. Each owner imports the runtime facade rather than its implementation leaves.

## Dependencies and platform boundary

The crate depends on `signal_artifacts` and portable third-party libraries. Native and browser host
implementations live in `logic_analyzer_platform` and are injected through capability contracts.
The same source modules and public contracts compile for native and wasm targets.

## Errors and tests

Runtime wiring and execution failures use the runtime owner's `ConnectionError`, `PortError`,
`WorkError`, and boundary `Error`. Capture and storage owners retain their more specific errors and
translate only when implementing a runtime trait. Unit and architecture tests cover both execution
behavior and dependency direction; platform repository conformance remains with the platform owner.

## Proposed future

The private runtime owner becomes an independent `signal_runtime` crate only after capture-specific
payload negotiation and level-stream classification are supplied through a dependency-safe generic
contract. Capture, derived-data, and capture-session extraction follows only when their internal
facades have equivalent one-way dependency checks.
