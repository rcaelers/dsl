# `logic_analyzer_graph_plan` Design

## Responsibility

`logic_analyzer_graph_plan` owns the immutable, execution-ready contract exchanged between graph
producers and consumers. `ProcessingGraph` contains resolved nodes, materializer handles, payload
capabilities, subscriptions, sampling metadata, retention policy, and concrete runtime edges.

## Boundary

The crate depends only on lower-level graph capabilities, node-graph identity, and signal-contract value
contracts. It has no compiler, registry, runtime-service, orchestration, UI, platform, concrete
node, or target dependency. It neither lowers documents nor starts work.

Compiler and runtime crates import this contract directly. Neither re-exports it as a convenience
facade.
