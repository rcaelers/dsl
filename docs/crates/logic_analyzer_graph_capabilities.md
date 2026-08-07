# `logic_analyzer_graph_capabilities` Design

## Responsibility

`logic_analyzer_graph_capabilities` owns the stable capability contracts exposed by graph nodes
and payload plugins to generic graph infrastructure. These include runtime materializers, port and
payload identities, resolved inputs, restricted build services, capture and trigger features, and
protocol-neutral presentation descriptors.

## Facade and dependencies

The crate exposes the directory-backed `node` and `node_support` namespaces. It depends only on
`node_graph_document`, the specific signal contract owners, and serialization support. Capability
methods receive the narrow `SocketReference` semantic identity rather than complete editor socket
records. It owns no inventory collection,
graph lowering, processing plan, runtime lifecycle, built-in node, UI state, or target selection.

Consumers import capability symbols from their owning namespace directly. Registry descriptors,
processing-plan values, and concrete node implementations remain in their separate owner crates.

Host catalog directories, persistence, and scanning are outside this crate. The UI owns the
portable catalog service contract, and application composition implements it by adapting generic
platform work mechanisms and the concrete scanner.
