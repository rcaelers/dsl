# `signal_processing` Design

## Responsibility

`signal_processing` owns portable generic signal contracts and the independently changing capture,
derived-data, and capture-session domains. It consumes immutable byte and artifact contracts
directly from `signal_artifacts`, and generic execution contracts from `signal_runtime`.
It owns no concrete device, capture format, protocol decoder, graph node, UI, host adapter, or
target selection.

## Runtime boundary

`signal_runtime` owns typed ports and channels, `ProcessNode`, pipeline construction, schedulers,
managers, watchdogs, runtime errors, and generic worker execution. This crate imports those
contracts directly and does not re-export them. Signal-specific capabilities such as `EdgeQuery`
adapt to the runtime's generic typed capability protocol in this crate.

## Other owners

The public `capture` and `waveform_index` facades own generic capture preparation, query, indexing,
and finite waveform behavior. `derived_word_store` and the private derived-data facades own payload
collection, sampling, indexing, and encoded derived storage. `live_capture`, `live_capture_store`,
and `logic_analyzer` own driver-neutral acquisition, session storage, policy, trigger, and capture
source contracts. Each owner imports `signal_runtime` directly.

## Dependencies and platform boundary

The crate depends on `signal_artifacts`, `signal_runtime`, and portable third-party libraries. Native and browser host
implementations live in `logic_analyzer_platform` and are injected through capability contracts.
The same source modules and public contracts compile for native and wasm targets.

## Errors and tests

Runtime wiring and execution failures use `signal_runtime` errors. Capture and storage owners retain
their more specific `Error` and translate only when implementing a runtime trait. Unit and
architecture tests cover signal-domain behavior and dependency direction; runtime execution tests
remain with `signal_runtime`.

## Proposed future

Capture, derived-data, and capture-session extraction follows only when their internal facades have
equivalent one-way dependency checks and consumers need independent compilation boundaries.
