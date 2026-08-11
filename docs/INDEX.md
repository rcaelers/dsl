# Architecture Documentation Index

This index contains one entry per narrative document. Public API details live at crate and
public-module facades in Rustdoc; planning documents are reached from the backlog items that use
them.

## Start here

- [Vocabulary and Concepts](architecture/vocabulary_and_concepts.md) — shared graph, runtime,
  capture, storage, and presentation terminology.
- [Crate Responsibilities](architecture/crate_responsibility.md) — positive ownership, principal
  handoffs, and workspace dependency direction.
- [Graph Composition Design](architecture/graph_composition.md) — graph capabilities, registries,
  built-in features, lowering, UI integration, and compatibility.
- [Processing Graph Workflows](architecture/processing_workflows.md) — lowering, execution,
  retained data, caching, sampling points, and live reconciliation.
- [Application Composition Design](architecture/application_composition.md) — native/web roots,
  host adaptation, application services, and graph interaction.

## Cross-cutting designs

- [Responsibility and Visibility Design](aspects/responsibility_visibility.md) — crate/module
  ownership, facades, supported public namespaces, and structural enforcement.
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md) — portable data
  planes, host mechanisms, source parity, and target-selection boundaries.
- [Live Capture and Trigger Control](aspects/live_capture_trigger.md) — acquisition lifecycle,
  trigger editing, storage publication, and UI coordination.
- [Plugin-Extensible Payload and Presentation Design](aspects/plugin_extensible_payload.md) —
  payload inventory, viewer lanes, and plug-in presentation contracts.
- [Performance Design and Measurement Record](aspects/performance.md) — reference workloads,
  acceptance rules, retained baselines, and measured/rejected experiments.
- [Testing Strategy](aspects/testing_strategy.md) — component, integration, architecture, platform,
  and hardware validation.

## Crate narratives

These pages explain cross-owner rationale or substantial internal domains that do not fit one
public API namespace. Rustdoc remains the consumer-facing contract reference.

### Runtime and data plane

- [platform_artifacts](crates/platform_artifacts.md) — byte sources, artifacts, repositories, and
  replication.
- [platform_runtime](crates/platform_runtime.md) — portable work execution and worker operations.
- [signal_runtime](crates/signal_runtime.md) — typed-stream execution and pipeline supervision.
- [signal_capture](crates/signal_capture.md) — immutable captures, queries, and finite indexes.
- [signal_derived](crates/signal_derived.md) — derived payloads, lanes, queries, and storage.
- [signal_capture_session](crates/signal_capture_session.md) — acquisition, recording, and capture
  source lifecycles.
- [Processing domain crates](crates/processing_domains.md) — transforms, sinks, generators, and
  protocol decoders.
- [logic_analyzer_capture_formats](crates/logic_analyzer_capture_formats.md) — DSL and Sigrok
  readers, indexes, and replay sources.
- [logic_analyzer_trigger](crates/logic_analyzer_trigger.md) — trigger schemas, programs, edits, and
  validation.
- [logic_analyzer_acquisition](crates/logic_analyzer_acquisition.md) — device-neutral acquisition
  and logic-analyzer source contracts.

### Graph and presentation

- [logic_analyzer_graph_capabilities](crates/logic_analyzer_graph_capabilities.md) — graph feature
  and payload contracts.
- [logic_analyzer_graph_registry](crates/logic_analyzer_graph_registry.md) — inventory validation
  and immutable capability catalogs.
- [logic_analyzer_graph_editor_registry](crates/logic_analyzer_graph_editor_registry.md) — product
  integration for reusable node-editor definitions.
- [logic_analyzer_graph_nodes](crates/logic_analyzer_graph_nodes.md) — built-in node features,
  migrations, builders, and presentation metadata.
- [logic_analyzer_graph_plan](crates/logic_analyzer_graph_plan.md) — immutable compiler/runtime
  processing-plan boundary.
- [logic_analyzer_graph_orchestration](crates/logic_analyzer_graph_orchestration.md) — graph worker
  protocol and compiler/runtime composition.
- [node_graph_document](crates/node_graph_document.md) — portable graph records and semantic
  identities.
- [node_graph](crates/node_graph.md) — reusable node-definition API and editor widget.
- [logic_analyzer_viewer](crates/logic_analyzer_viewer.md) — waveform and derived-lane viewer.

### Applications and integration tests

- [Application shells](crates/application_shells.md) — native/web composition roots and host
  adapters.
- [Workspace examples and integration tests](crates/logic_analyzer_examples.md) — reference graphs,
  cross-crate checks, benchmarks, and performance regression tooling.

## External integrations

- [Sigrok Python Decoder Host](integrations/sigrok_python_decoder.md) — embedded decoder execution
  and host contract.
- [Sigrok Decoder Distribution](integrations/sigrok_decoder_distribution.md) — decoder discovery,
  packaging, and licensing policy.
- [DSLogic U3Pro16 Protocol](integrations/dslogic_u3pro16_protocol.md) — hardware protocol and
  acquisition sequences.

## Supporting references and project work

- [CCD AFE register map](references/ccd_afe_registers.md) — standalone CCD framebuffer hardware
  reference.
- [Project backlog](../TODO.md) — proposed features, refactorings, performance work, and links to
  active implementation plans.

## Public API documentation

Generate the supported crate and public-module API locally with:

```sh
cargo doc --workspace --no-deps --lib --open
```

The public-module allowlist is maintained by Responsibility and Visibility Design. Narrative crate
pages remain only for relationships that span public contracts; actionable work remains in the
project backlog.
