# `logic_analyzer_test_support` Design

## Responsibility

`logic_analyzer_test_support` owns deterministic capture providers and data-plane conformance
fixtures reused by cross-crate tests.

## Facade and dependencies

Its root exposes fake-provider controls and repository, capture-store, and derived-store snapshots.
It depends only on generic `signal_processing` contracts.

## Ownership boundaries

Production composition, concrete processing nodes, graph-node definitions, UI fakes, and host
adapters do not belong here. Component tests own local fakes at their own service boundaries;
workspace integration tests own full compositions.
