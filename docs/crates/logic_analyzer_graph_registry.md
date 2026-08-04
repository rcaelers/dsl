# `logic_analyzer_graph_registry` Design

## Responsibility

`logic_analyzer_graph_registry` owns graph-node and payload registration descriptors, inventory
collection, deterministic validation, host runtime-builder override resolution, and the immutable
`GraphRegistry` snapshot consumed by the compiler and UI composition.

## Facade and dependencies

The crate root exposes `GraphNodeRegistration`, `PayloadRegistration`, their validated inventory
iterators, and `GraphRegistry`. It depends only on `logic_analyzer_graph_capabilities`, `node_graph`,
`signal_processing`, and the generic `inventory` mechanism. Plugins implement graph-API capability
traits and submit registry-owned descriptors.

## Ownership boundaries

The registry does not own editable graph documents, output-subscription plans, compiler lowering,
generated collector nodes, runtime lifecycles, concrete graph nodes, UI presentation policy, or
target selection. A consumer may inject neutral infrastructure builders while constructing a
snapshot, but those builders and their policy remain owned by that consumer.

Graph-node registration validation rejects empty or duplicate stable IDs and duplicate definition
names. Payload registration validation rejects empty or duplicate stable IDs and duplicate runtime
types. Snapshot construction also validates graph-node payload requirements and rejects unresolved
host overrides or collisions with consumer-supplied infrastructure builders.

## Compatibility

Moving registration ownership does not change inventory linkage, stable graph-node IDs, payload
IDs, builder names, serialized graph state, or graph extensions. Enabled built-in and plugin crates
retain their existing linker anchors.

## Test boundary

Registry tests cover deterministic validation, dependency direction, and facade policy. Built-in
feature contract tests remain with `logic_analyzer_graph_nodes`; compiler tests consume the public
immutable registry rather than reading inventory directly.
