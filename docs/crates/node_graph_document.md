# `node_graph_document` Design

## Responsibility

`node_graph_document` owns the portable persisted graph document: graph, node, socket,
connection, and frame records; stable node and socket identities; neutral colors and positions;
serialization; semantic snapshots; and document-local editing invariants such as variadic socket
indexing and reroute reconciliation.

The records contain no egui types. `GraphColor` preserves the serialized RGBA byte-array shape and
`GraphPosition` preserves the serialized `{ "x", "y" }` shape used by existing saved graphs. The
`node_graph` widget converts these values to egui types only at its rendering and interaction
boundary.

## Semantic socket contract

`Socket` remains the complete persisted record because the editor must save its generic schema and
presentation state. Compiler-facing capability contracts receive `SocketReference` instead. A
reference exposes only the stable schema ID, definition slot, variadic member position, and
direction. Labels, colors, shape, visibility, controls, and extension data therefore cannot become
execution inputs through the capability interface.

`Node::socket_reference` is the canonical projection. It derives variadic member positions from
the materialized document so compiler, UI discovery, and tests share one indexing rule.

## Facade and dependencies

The crate exposes its supported records through its crate root and has no public modules. It
depends only on serde and JSON values, with no workspace-crate, widget, runtime, compiler, UI, or
host dependency. `node_graph` re-exports the records for source compatibility, while headless graph
crates depend on this owner directly.

## Compatibility boundary

Saved-document serde shape is an explicit contract. A checked-in graph fixture is deserialized and
reserialized by this crate and compared as JSON. Definition-owned reconciliation and migrations
remain at application load through the node editor registry; the neutral document crate does not
infer concrete node behavior.
