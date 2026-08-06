# Application Shells Design

## Responsibility

`logic-analyzer-app-native` and `logic-analyzer-app-web` are composition roots. They obtain
host-selected mechanisms, retain enabled inventory bundles, adapt UI/domain ports, select concrete
node capabilities, and construct the portable UI application and worker runtime.

## Boundaries

They own entry/bootstrap code and application-specific adaptation of host mechanisms. They do not
own generic storage, source preparation, graph lowering, processing execution, capture policy, or
reusable widgets. Their target-specific dependencies are permitted because they are application
roots; reusable low-level host mechanisms remain in `logic_analyzer_platform`.

## Host bootstrap and documents

`logic-analyzer-app-native` provides the `logic-conduit` binary, its CLI, logging setup, and the
native eframe window. Its `run <graph.json>` command composes the same `AppServices` without a
window, restores the graph, prepares its source, applies its saved output-retention and cursor
contracts, executes the run, and reports preparation, cache removal, execution, and total time.

`logic-analyzer-app-web` exports the wasm-bindgen `WebHandle`. The browser shell supplies the
generated JavaScript-module and WASM URLs used by `logic_analyzer_platform` to provide worker
transport or its cooperative fallback. The web app constructs the worker graph runtime and adapts
platform byte-oriented document handles and downloads to UI graph-document operations. Native
composition provides filesystem paths and dialogs. Opaque selected document references last for
the page session; stale references are not offered after reload.

The native application loads user-selected graphs. The web application embeds a named demo catalog
and exposes it through Demos. The first entry is the self-contained SPI-controlled parallel-bus
capture in `crates/app_web/data/wasm_decoder_demo.json`; every `graphs/*_demo.json` is also
embedded. A web-crate test keeps that catalog synchronized. Editable examples remain independent
of code, while programmatic graph fixtures and file-backed test data remain with their owning test
crate.

## Web composition

The shared `App` compiles to `wasm32-unknown-unknown`. A selected demo graph runs through the
cooperative manager pumped by the frame loop, with the same graph-node definitions and builders as
native. Web application composition installs unavailable USB, export, and output-destination
capabilities explicitly; it never substitutes synthetic input or discard output because of the
target. Synthetic demo data is selected only by saved node configuration.
