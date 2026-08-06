# `platform_runtime` Design

## Responsibility

`platform_runtime` owns platform-neutral host execution contracts: bounded and long-running work,
completion tasks, serializable worker-operation messages, worker capabilities, kernel registration,
and the deterministic queue shared by native and browser worker adapters. It also supplies explicit
inline and cooperative fallback implementations.

It does not own typed stream graphs, process nodes, pipeline lifecycle, signal payloads, storage,
application policy, or target-specific transports.

## Public facade

The crate root is the only supported facade. Consumers import work and worker contracts directly
from `platform_runtime`; `signal_runtime` and other higher-level crates do not re-export them.

## Dependencies and boundary

The crate depends only on portable serialization and error support. It has no dependency on signal,
graph, processing, UI, application, or adapter crates and contains no target selection.
`logic_analyzer_platform` implements its contracts with native threads and browser workers.
Application roots inject those implementations into `signal_runtime` and the other consumers that
schedule work.

## Invariants and tests

Worker operation identifiers and payloads are owned, portable values. The queue bounds outstanding
work, accepts monotonically increasing sequence identifiers, and releases terminal results in
submission order. Native and wasm tests cover message round trips, backpressure, completion,
cancellation, and worker failure. Workspace dependency tests keep both platform contract owners
independent of product and signal-domain crates.
