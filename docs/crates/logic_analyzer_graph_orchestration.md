# `logic_analyzer_graph_orchestration` Design

## Responsibility

`logic_analyzer_graph_orchestration` owns application-neutral workflows that necessarily compose
graph compilation and runtime execution. Its current public surface is the bounded graph-worker
request/message protocol, codec, client, and worker runtime.

## Boundary

The worker runtime lowers the request's editable graph with `GraphLowerer`, then passes the
resulting `ProcessingGraph` to a separate `GraphRuntime`. This above-layer composition does not
place a dependency between compiler and runtime. Target-selected worker creation and transport
remain in `logic_analyzer_platform`; ordinary Run orchestration remains in the UI's private graph
service.
