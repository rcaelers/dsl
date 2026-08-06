# `signal_derived` Design

## Responsibility

`signal_derived` owns presentation-neutral retained outputs: derived payload records, open payload
registration, collected-lane ingestion and queries, sampling points, mipmap indexes, and encoded
artifact-backed word storage. Its crate root is the supported facade; `derived_word_store` is its
only public module.

## Dependency boundary

The crate depends on `platform_artifacts`, `platform_runtime`, `signal_capture`, `signal_runtime`,
and portable support libraries. It has no dependency on acquisition sessions, graph crates,
concrete protocols or nodes, viewers, UI, or platform adapters. Digital lanes consume the generic
`Sample` payload from `signal_capture`; collectors implement generic `signal_runtime` process-node
contracts, while background encoding uses `platform_runtime` work and worker-operation contracts.

Shared persistence time and integrity primitives come from `platform_artifacts`, so derived and
capture-session stores do not depend on each other.

## Errors and tests

Payload registration reports identity collisions explicitly. Encoded stores retain codec, query,
and persistence-specific errors. Unit tests cover every built-in generic adapter, bounded snapshots,
encoded storage, cache behavior, sampling points, and worker kernels. Architecture tests reject
session, graph, UI, and concrete-protocol dependencies.
