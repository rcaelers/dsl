# `logic_analyzer_capture_export` Design

## Responsibility

`logic_analyzer_capture_export` streams finalized generic capture storage to supported export
formats. It owns export progress, cancellation observation, reports, and archive encoding.

## Facade and dependencies

The root exports descriptors, formats, progress/report types, and the export operation. It depends
on `signal_processing` capture contracts and format libraries, not graph, node, UI, or platform
crates.

## Ownership boundaries

Destination acquisition and asynchronous UI orchestration belong to the UI host-service contract;
target-specific service selection belongs to `logic_analyzer_platform`. This crate never decides
whether an export destination is available.
