# Architecture Documentation Index

## Workspace structure

- [Vocabulary and Concepts](architecture/vocabulary_and_concepts.md) defines the shared terms and
  identities used by graph, runtime, capture, storage, and presentation designs.
- [Crate Responsibilities](architecture/crate_responsibility.md) describes every workspace crate,
  its relevant public modules, its ownership boundary, and dependency direction.
- [Responsibility and Visibility Design](aspects/responsibility_visibility.md) defines module
  facades, public API visibility, platform ownership, and enforcement.
- [Graph Composition Design](architecture/graph_composition.md) defines graph capabilities, registry,
  built-in-node, compiler, UI, and compatibility ownership.
- [Processing Graph Workflows](architecture/processing_workflows.md) explains lowering, capability
  negotiation, Run, retained derived data, cache reuse, sampling points, and live reconciliation.
- [Application Composition Design](architecture/application_composition.md) defines application
  composition and graph interaction.

## Cross-cutting aspects

- [Responsibility and Visibility Design](aspects/responsibility_visibility.md)
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md)
- [Live Capture and Trigger Control](aspects/live_capture_trigger.md)
- [Plugin-Extensible Payload and Presentation Design](aspects/plugin_extensible_payload.md)
- [Performance Design and Measurement Record](aspects/performance.md) defines the reference
  workloads, the acceptance rule, the retained baseline, and the experiments performed, including
  the approaches that were measured and rejected.
- [Testing Strategy](aspects/testing_strategy.md)

## Public API documentation

The supported public API is documented at its crate and public-module facade with Rustdoc.
Generate it locally with:

```sh
cargo doc --workspace --no-deps --lib --open
```

The owner pages below remain narrative companions: Rustdoc is the entry point for consumers and
plugin authors, while these pages retain longer design rationale that spans public contracts.

## Cross-component crate narratives

The remaining crate narratives describe relationships that cannot belong to one public API
namespace. All other crate contracts are documented in Rustdoc.

- [logic_analyzer_graph_nodes](crates/logic_analyzer_graph_nodes.md)
- [logic_analyzer_graph_capabilities](crates/logic_analyzer_graph_capabilities.md)
- [logic_analyzer_graph_registry](crates/logic_analyzer_graph_registry.md)
- [logic_analyzer_graph_plan](crates/logic_analyzer_graph_plan.md)
- [logic_analyzer_graph_orchestration](crates/logic_analyzer_graph_orchestration.md)
- [node_graph](crates/node_graph.md)
- [logic_analyzer_viewer](crates/logic_analyzer_viewer.md)
- [Application shells](crates/application_shells.md)
- [Workspace examples and integration tests](crates/logic_analyzer_examples.md)
- [platform_artifacts](crates/platform_artifacts.md)
- [platform_runtime](crates/platform_runtime.md)
- [signal_capture](crates/signal_capture.md)
- [signal_derived](crates/signal_derived.md)
- [signal_capture_session](crates/signal_capture_session.md)
- [signal_runtime](crates/signal_runtime.md)
- [Processing domain crates](crates/processing_domains.md)

## Public modules

These supported namespaces are documented at their source facades in Rustdoc:

- `platform_artifacts`: no public modules; its crate root is the artifact-contract facade
- `platform_runtime`: no public modules; its crate root is the host-execution-contract facade
- `signal_capture`: no public modules; its crate root is the immutable capture facade
- `signal_derived`: `derived_word_store`
- `signal_capture_session`: `live_capture`, `live_capture_store`, and `logic_analyzer`
- `signal_runtime`: no public modules; its crate root is the execution-contract facade
- `logic_analyzer_capture_formats`: `dsl_file` and `sigrok_file`
- `logic_analyzer_device_dslogic`: no public modules; its crate root is the device facade
- `logic_analyzer_protocol_decoders`: `i2c_decoder`, `parallel_decoder`, `sigrok_decoder`,
  `spi_decoder`, `types`, and `uart_decoder`
- `signal_transforms`: one public module per transform
- `signal_sinks`: one public module per sink
- `signal_generators`: `synthetic_capture_source` and `synthetic_uart_source`
- `logic_analyzer_graph_capabilities`: `node` and `node_support`
- `node_graph`: `api`

## Generic runtime and data plane

- [`platform_artifacts` Design](crates/platform_artifacts.md)
- [`platform_runtime` Design](crates/platform_runtime.md)
- [`signal_capture` Design](crates/signal_capture.md)
- [`signal_derived` Design](crates/signal_derived.md)
- [`signal_capture_session` Design](crates/signal_capture_session.md)
- [`signal_runtime` Design](crates/signal_runtime.md)
- [`signal_runtime` Rustdoc](../crates/signal_runtime/src/lib.rs)
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md)
- [`signal_derived::derived_word_store` Rustdoc](../crates/signal_derived/src/derived_word_store/mod.rs)
- [Live Capture and Trigger Control](aspects/live_capture_trigger.md)

## Reusable widgets and presentation

- [`node_graph` Design](crates/node_graph.md)
- [`logic_analyzer_viewer` Design](crates/logic_analyzer_viewer.md)
- [Plugin-Extensible Payload and Presentation Design](aspects/plugin_extensible_payload.md)

## External integrations

- [Sigrok Python Decoder Host](integrations/sigrok_python_decoder.md)
- [Sigrok Decoder Distribution](integrations/sigrok_decoder_distribution.md)
- [DSLogic U3Pro16 Protocol](integrations/dslogic_u3pro16_protocol.md)

## Quality and supporting references

- [Testing Strategy](aspects/testing_strategy.md)
- [Performance Design and Measurement Record](aspects/performance.md)
- [Project backlog](../TODO.md)
- [CCD AFE register map](references/ccd_afe_registers.md)

## Owner-document convention

Each non-trivial public crate and allowlisted public module documents its supported contract at its
source facade through Rustdoc. A crate-owner or module narrative in `docs/` remains only when it
explains cross-owner rationale, a substantial internal domain, or a relationship that cannot belong
to one API namespace. Those documents describe only implemented architecture in present tense and
link back to the aspect designs above. Actionable work belongs in `TODO.md`.
