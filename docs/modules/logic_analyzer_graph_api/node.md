# `logic_analyzer_graph_api::node`

## Responsibility

This namespace owns traits and inventory registrations implemented by graph-node and payload
plugins, including runtime builders, graph-node registrations, payload registrations, and capture
feature factories.

## Boundaries

It is an extension contract, not a compiler, node bundle, UI service, or host adapter. Implementers
use `node_support` for all supporting values and do not depend on concrete compiler internals. Its
current `DirectoryNodeCatalog` path configuration is the documented host-path exception scheduled
to move behind UI and platform ownership.
