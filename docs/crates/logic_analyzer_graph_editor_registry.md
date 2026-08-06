# `logic_analyzer_graph_editor_registry` Design

## Responsibility

`logic_analyzer_graph_editor_registry` owns the Logic Conduit integration between stable graph
feature IDs and `node_graph` editor definitions. Plug-ins submit one
`GraphNodeEditorRegistration` for each editor definition and submit the corresponding headless
capabilities to `logic_analyzer_graph_registry` under the same stable ID.

The crate also owns `GraphNodeEditorOverride`, the composition-time mechanism for replacing a
default definition registration with an instance-bound registration that captures an injected host
service. The UI joins editor and headless inventories by stable ID, validates matching names and
complete coverage, then applies overrides while building its `NodeTypeRegistry`.

## Boundary

This is an editor integration crate and therefore depends on `node_graph`. The headless registry,
capabilities, compiler, plan, runtime, and worker orchestration do not depend on it. It owns no
graph document records, node behavior, compiler policy, runtime lifecycle, or host adapter.

The crate root is the complete facade; implementation modules remain private.
