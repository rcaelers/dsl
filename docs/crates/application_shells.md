# Application Shells Design

## Responsibility

`logic-analyzer-app-native` and `logic-analyzer-app-web` are thin composition roots. They create
the host-selected platform services, retain enabled inventory bundles, and construct the portable
UI application.

## Boundaries

They own only native or web entry/bootstrap code. They do not own generic storage, source
preparation, graph lowering, processing execution, capture policy, or reusable widgets. Their
target-specific dependencies are permitted only because they are application roots; reusable host
adapters remain in `logic_analyzer_platform`.

## Host bootstrap and documents

`logic-analyzer-app-native` provides the `logic-conduit` binary, its CLI, logging setup, and the
native eframe window. Its `run <graph.json>` command composes the same `AppServices` without a
window, restores the graph, prepares its source, applies its saved output-retention and cursor
contracts, executes the run, and reports preparation, cache removal, execution, and total time.

`logic-analyzer-app-web` exports the wasm-bindgen `WebHandle`. The browser shell supplies the
generated JavaScript-module and WASM URLs used by `logic_analyzer_platform` to provide a worker
or its cooperative fallback. Native hosts provide filesystem paths and dialogs. The web host keeps
opaque selected document references for its page session and implements Save and Save As as named
JSON downloads; stale references are not offered after reload.

The native application loads user-selected graphs. The web application embeds a named demo catalog
and exposes it through Demos. The first entry is the self-contained SPI-controlled parallel-bus
capture in `crates/app_web/data/wasm_decoder_demo.json`; every `graphs/*_demo.json` is also
embedded. A web-crate test keeps that catalog synchronized. Editable examples remain independent
of code, while programmatic graph fixtures and file-backed test data remain with their owning test
crate.

## Web composition

The shared `App` compiles to `wasm32-unknown-unknown`. A selected demo graph runs through the
cooperative manager pumped by the frame loop, with the same graph-node definitions and builders as
native. Platform composition advertises unavailable file, USB, export, and output-destination
capabilities explicitly; it never substitutes synthetic input or discard output because of the
target. Synthetic demo data is selected only by saved node configuration.
