# Processing Domain Crates

## Responsibility split

UI-independent processing behavior is divided by positive responsibility:

- `logic_analyzer_capture_formats` owns DSL and Sigrok archive parsing, indexes, and replay
  sources;
- `logic_analyzer_trigger` owns portable trigger programs, schemas, and validation;
- `logic_analyzer_acquisition` owns shared logic-analyzer driver, capture-configuration, and
  runtime-source contracts;
- `logic_analyzer_device_dslogic` owns the DSLogic U3Pro16 protocol and acquisition source;
- `logic_analyzer_protocol_decoders` owns protocol packet values, packet framing, concrete
  decoded-protocol state machines, and decoder host contracts;
- `signal_transforms` owns portable stream transformations;
- `signal_sinks` owns terminal consumers and output encodings; and
- `signal_generators` owns explicit deterministic signal sources.

These crates implement or consume contracts from `signal_runtime`, `signal_capture`,
`signal_derived`, and `signal_capture_session`. None owns graph definitions, saved graph state,
socket presentation, application composition, or host selection. `logic_analyzer_graph_nodes`
binds their runtime behavior to concrete graph features.

## Shared construction contracts

`signal_runtime::ProcessNodeConstruction<M>` is the neutral factory result for a process plus
owner-defined metadata. Lazy capture-source lifecycle, presentation, cache identity, and
acquisition metadata belong to `signal_capture_session`. Concrete source factories import those
contracts directly instead of routing through another processing crate.

## Host capabilities

Capture formats consume prepared byte sources and injected artifact/work services. File-I/O
compatibility leaves are explicitly allowlisted. The DSLogic crate consumes its own neutral USB
transport and FPGA-image capability; `platform` supplies the native adapter. Decoder, transform,
sink, and generator crates contain no target selection.

The top-level `logic-analyzer-examples` package owns the parallel-decoder, SPI-decoder, and U3Pro16
streaming benchmark binaries. Reusable processing crates therefore carry no CLI or logging-subscriber
dependency for developer tools.
