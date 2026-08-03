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

## Crate owners

- [signal_processing](crates/signal_processing.md)
- [logic_analyzer_processing](crates/logic_analyzer_processing.md)
- [logic_analyzer_graph_api](crates/logic_analyzer_graph_api.md)
- [logic_analyzer_graph_compiler](crates/logic_analyzer_graph_compiler.md)
- [logic_analyzer_graph_nodes](crates/logic_analyzer_graph_nodes.md)
- [logic_analyzer_platform](crates/logic_analyzer_platform.md)
- [logic_analyzer_ui](crates/logic_analyzer_ui.md)
- [logic_analyzer_capture_export](crates/logic_analyzer_capture_export.md)
- [logic_analyzer_test_support](crates/logic_analyzer_test_support.md)
- [node_graph](crates/node_graph.md)
- [logic_analyzer_viewer](crates/logic_analyzer_viewer.md)
- [input_bindings](crates/input_bindings.md), [panel_layout](crates/panel_layout.md),
  [trigger_editor](crates/trigger_editor.md), and [widget_support](crates/widget_support.md)
- [Application shells](crates/application_shells.md)
- [Example plugin](crates/example_plugin.md) and
  [workspace examples and integration tests](crates/logic_analyzer_examples.md)

## Public module owners

- `signal_processing`: [capture](modules/signal_processing/capture.md),
  [live capture](modules/signal_processing/live_capture.md),
  [live-capture store](modules/signal_processing/live_capture_store.md),
  [logic-analyser contracts](modules/signal_processing/logic_analyzer.md),
  [derived-word store](modules/signal_processing/derived_word_store.md), and
  [waveform index](modules/signal_processing/waveform_index.md)
- `logic_analyzer_processing`: [nodes](modules/logic_analyzer_processing/nodes.md) and
  [types](modules/logic_analyzer_processing/types.md); the node-owner page indexes every public
  family and concrete processing node.
- `logic_analyzer_graph_api`: [node](modules/logic_analyzer_graph_api/node.md) and
  [node support](modules/logic_analyzer_graph_api/node_support.md)
- `node_graph`: [API](modules/node_graph/api.md)

## Generic runtime and data plane

- [`signal_processing` Runtime Design](modules/signal_processing/runtime.md)
- [Unified Native and Web Storage Platform Design](aspects/native_web_storage.md)
- [`signal_processing::derived_word_store` Design](modules/signal_processing/derived_word_store.md)
- [Live Capture and Trigger Control](aspects/live_capture_trigger.md)

## Reusable widgets and presentation

- [`node_graph` Design](crates/node_graph.md)
- [Node Graph Widget API](crates/node_graph_api.md)
- [`logic_analyzer_viewer` Design](crates/logic_analyzer_viewer.md)
- [Logic Analyzer Viewer API](crates/logic_analyzer_viewer_api.md)
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

Each non-trivial crate has one owner design at `docs/crates/<crate>.md`. A public module or
independently-owned internal domain receives a module design at
`docs/modules/<crate>/<module>.md` when its contract needs detail beyond the crate owner document.
Those documents describe the architecture in present tense and link back to the aspect designs
above; proposed-future architecture is clearly labeled and actionable work remains in `TODO.md`.
