# `signal_capture_session` Design

## Responsibility

`signal_capture_session` owns the portable generic capture-session domain: acquisition lifecycle,
append-only live storage, capture policy, trigger programs, and driver-neutral capture control. It
consumes immutable capture from `signal_capture`, retained outputs from `signal_derived`, artifact
contracts from `platform_artifacts`, and execution from `signal_runtime`.
Host work scheduling comes from `platform_runtime`.
It owns no concrete device, capture format, protocol decoder, graph node, UI, host adapter, or
target selection.

## Runtime boundary

`signal_runtime` owns typed ports and channels, `ProcessNode`, pipeline construction, schedulers,
managers, watchdogs, and process-node errors. `platform_runtime` owns host work execution. This
crate imports those contracts directly and does not re-export them. `signal_capture` owns the `EdgeQuery` adapter to
the runtime's generic typed capability protocol.

## Session owners

`live_capture`, `live_capture_store`, and `logic_analyzer` own driver-neutral acquisition, session
storage, policy, trigger, and capture-source contracts. Growing waveform indexes remain here because
they consume mutable session storage. `GrowingCaptureIndex` follows committed chunks in the
authoritative live store and implements the same `CaptureIndex` sampled-window and transition-query
contract as finite indexes. It reuses `signal_capture::WaveformSummaryGrid` and the shared
resolution-selection algorithm; finite artifact layout, raw-block caching, and immutable index
publication remain in `signal_capture`.

## Dependencies and platform boundary

The crate depends on `platform_artifacts`, `platform_runtime`, `signal_capture`, `signal_derived`,
`signal_runtime`, and portable third-party libraries. Native and browser host
implementations live in `platform` and are injected through capability contracts.
The same source modules and public contracts compile for native and wasm targets.

## Errors and tests

Runtime wiring and execution failures use `signal_runtime` errors. Capture and storage owners retain
their more specific `Error` and translate only when implementing a runtime trait. Unit and
architecture tests cover signal-domain behavior and dependency direction; runtime execution tests
remain with `signal_runtime`.
