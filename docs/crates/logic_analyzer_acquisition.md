# `logic_analyzer_acquisition` Design

## Responsibility

`logic_analyzer_acquisition` owns device-neutral logic-analyzer driver contracts, capture
configuration, raw chunk encoding, hardware-trigger lowering values, and the generic runtime source
adapter for a `LogicAnalyzer` implementation.

Concrete device protocols and transports remain in device crates. Capture-session lifecycle,
bounded recording, retention, and storage remain in `signal_capture_session`. Trigger programs and
capability schemas remain in `logic_analyzer_trigger`; graph definitions and saved state remain in
`logic_analyzer_graph_nodes`.

## Runtime boundary

The runtime source adapter turns an injected driver into typed edge, packed-block, and raw-chunk
outputs using `signal_runtime` and injected `platform_runtime` work execution. It neither selects a
host transport nor creates a capture session. Native and web composition therefore share the same
source contract while concrete host availability is decided at the application boundary.
