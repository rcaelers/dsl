# `signal_runtime` Design

## Responsibility

`signal_runtime` owns UI-independent typed-stream execution: port and payload contracts, channel
transport, process nodes, pipelines, threaded and cooperative managers, scheduling, watchdogs, and
application-manager facades. Its crate root is the supported facade; implementation modules are
private.

## Dependency boundary

The crate depends on the platform-neutral `platform_runtime` work capability and portable
concurrency, serialization, timing, error, and tracing libraries. It has no dependency on
`platform_artifacts`, graph crates, concrete
nodes, storage, capture sessions, protocols, UI, or platform adapters. Payload negotiation uses
generic type identity plus explicit stream semantics and buffer metadata. Non-stream transports use
generic typed protocol capabilities.

Signal domains define their own payload records and capability adapters. In particular,
`signal_capture` owns `EdgeQuery` and adapts it to `ProtocolCapability`; the runtime does not know
the query's boolean-signal semantics.

## Errors and tests

`ConnectionError` and `PortError` report graph wiring failures. `WorkError` is the process-node
execution boundary. Unit tests cover channel behavior, negotiation, scheduling, managers, worker
shutdown, and injected execution. Worker dispatch and cancellation are tested by their
`platform_runtime` owner. Architecture tests reject signal-domain and storage
dependencies.
