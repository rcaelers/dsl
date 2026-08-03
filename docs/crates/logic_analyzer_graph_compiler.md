# `logic_analyzer_graph_compiler` Design

## Responsibility

`logic_analyzer_graph_compiler` converts an editor graph plus explicit output subscriptions into a
validated runtime graph and currently owns the resulting graph-run lifecycle. It validates
inventory registrations, resolves ports and payload kinds, plans generated collectors and caches,
prepares sources, and drives live runs through `signal_processing`.

## Facade and dependencies

The crate-root façade exposes `GraphCompiler`, `CompiledGraph`, diagnostics, source-preparation
types, worker protocol types, and `LiveRun`. It depends on graph API and document contracts plus
generic signal-processing contracts; it has no dependency on concrete graph nodes, processing
implementations, widgets, or UI.

## Ownership boundaries

The compiler does not persist UI selection state, construct editor registries, infer protocol
presentation, or select a host target. The compiler/execution combination is the primary
responsibility split proposed in [Crate Responsibility Design](../architecture/crate_responsibility.md);
until that split lands, this document records the implemented combined façade.
