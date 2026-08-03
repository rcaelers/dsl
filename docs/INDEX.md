# Architecture Documentation Index

## Workspace structure

- [Crate Responsibility Design](architecture/crate_responsibility.md) defines the responsibility map,
  dependency direction, proposed graph-runtime boundary, and proposed decomposition of generic
  processing infrastructure.
- [Responsibility and Visibility Design](aspects/responsibility_visibility.md) defines module
  facades, public API visibility, platform ownership, and enforcement.
- [Graph Composition Design](architecture/graph_composition.md) defines graph API, built-in-node,
  compiler, UI, and compatibility ownership.
- [Application Composition Design](architecture/application_composition.md) defines application
  composition and graph interaction.

## Cross-cutting aspects

- [Responsibility and Visibility Design](aspects/responsibility_visibility.md)
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md)
- [Live Capture and Trigger Control](aspects/live_capture_trigger.md)
- [Plugin-Extensible Payload and Presentation Design](aspects/plugin_extensible_payload.md)
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
- [node_graph](crates/node_graph.md)
- [logic_analyzer_viewer](crates/logic_analyzer_viewer.md)
- [Application shells](crates/application_shells.md)
- [Workspace examples and integration tests](crates/logic_analyzer_examples.md)

## Public modules

These supported namespaces are documented at their source facades in Rustdoc:

- `signal_processing`: `capture`, `live_capture`, `live_capture_store`, `logic_analyzer`,
  `derived_word_store`, and `waveform_index`
- `logic_analyzer_processing`: `nodes`, `types`, each node family, and each concrete node
- `logic_analyzer_graph_api`: `node` and `node_support`
- `node_graph`: `api`

## Generic runtime and data plane

- [`signal_processing` runtime Rustdoc](../crates/signal_processing/src/lib.rs)
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md)
- [`signal_processing::derived_word_store` Rustdoc](../crates/signal_processing/src/derived_word_store/mod.rs)
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
- [Project backlog](../TODO.md)
- [CCD AFE register map](references/ccd_afe_registers.md)

## Owner-document convention

Each non-trivial public crate and allowlisted public module documents its supported contract at its
source facade through Rustdoc. A crate-owner or module narrative in `docs/` remains only when it
explains cross-owner rationale, a substantial internal domain, or a relationship that cannot belong
to one API namespace. Those documents describe the architecture in present tense and link back to
the aspect designs above; proposed-future architecture is clearly labeled and actionable work
remains in `TODO.md`.
