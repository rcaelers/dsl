# `logic_analyzer_graph_registry` Design

## Responsibility

`logic_analyzer_graph_registry` owns graph-node and payload registration descriptors, inventory
collection, deterministic validation, host capability-override resolution, and the immutable
`GraphRegistry` snapshot consumed by the compiler and UI composition. It also owns protocol-packet
presentation registration and inventory lookup while graph capabilities retain only the display
value contract.

## Facade and dependencies

The crate root exposes `GraphNodeRegistration`, `PayloadRegistration`,
`ProtocolPacketPresentationRegistration`, their inventory lookups, and `GraphRegistry`. It depends only on `logic_analyzer_graph_capabilities`, `node_graph`,
the capture and derived contract owners, and the generic `inventory` mechanism. Plugins implement graph-API capability
traits and submit registry-owned descriptors.

## Ownership boundaries

The registry does not own editable graph documents, output-subscription plans, compiler lowering,
generated collector nodes, runtime lifecycles, concrete graph nodes, UI presentation policy, or
target selection. A consumer may inject neutral infrastructure capability bundles while
constructing a snapshot, but those capabilities and their policy remain owned by that consumer.

Graph-node registration validation rejects empty or duplicate stable IDs and duplicate definition
names. Payload registration validation rejects empty or duplicate stable IDs and duplicate runtime
types. Snapshot construction also validates graph-node payload requirements and rejects unresolved
host overrides or collisions with consumer-supplied infrastructure capabilities.

## Stable identities and deterministic lookup

Stable graph-node IDs and payload IDs are persisted feature identities. Definition names select
node behavior in graph documents, while renderer and protocol-presentation keys select registered
presentation behavior. Enabled built-in and plug-in crates expose linker anchors so every intended
inventory submission is retained before deterministic collection.

## Test boundary

Registry tests cover deterministic validation, dependency direction, and facade policy. Built-in
feature contract tests remain with `logic_analyzer_graph_nodes`; compiler tests consume the public
immutable registry rather than reading inventory directly.
