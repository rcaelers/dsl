# TODO

Task IDs start with their ownership category and remain stable when task wording changes.

## User-visible features

### Logic-analyzer viewer

- [viewer.presentation-colors] Add viewer presentation color controls, starting with a separately configurable color for
  each sampling overlay so simultaneous decoder sampling points remain distinguishable. Extend the same generic color
  contract to cursors, timeline markers, measurements, and other annotations where useful; keep defaults theme-owned,
  persist overrides by stable item identity, and avoid protocol-specific color handling in the viewer.
- [viewer.multiple-sources] Support displaying multiple capture sources in the logic-analyzer viewer.
- [viewer.source-selection] Let the viewer select which source is visible while the one-source display restriction
  remains.
- [viewer.source-alignment] Add time offsets and alignment controls for sources, including a clear shared time-base
  model.
- [viewer.live-snapshots] Display live-source snapshots in the viewer through the same `CaptureDataSource` boundary
  used by file captures.

### Capture sources

- [capture.live.segmented-acquisition] Introduce repeated and segmented acquisition with frame identity, per-frame origin and trigger
  metadata, bounded storage, replay, and viewer navigation.
- [capture.live.partial-analysis] Add live search and measurements over committed raw/derived prefixes with explicit coverage and
  lag.
- [capture.live.automation-service] Expose the same validated coordinator commands and outcomes through a UI-independent automation
  service.
- [capture.live.external-timing] Add external trigger/clock contracts and shared-timeline alignment after multi-source viewer
  support is defined.
- [capture.live.snapshot-persistence] Persist/reload live-capture snapshots where appropriate so they can be indexed and revisited.
- [capture.sigrok.extended-formats] Extend Sigrok support beyond v2 digital `logic-*` data (analog channels and newer format versions).

### Web platform (lower priority)

- [capture.web.file-import] Let the web application open user-selected and drag-and-dropped capture files through
  the platform-neutral prepared-source contract. Keep browser handles and permission flow in the web host adapter;
  materialize bounded files into chunked memory first, then add worker-owned or OPFS-backed access for larger files.
- [capture.web.file-export] Let web users export captures and generated files through an explicit destination acquired
  by a user gesture. Keep downloads separate from internal cache publication and report unsupported or lost
  permissions without changing processing-node behavior.
- [capture.web.usb] Investigate and, where the browser and device permit it, add U3Pro16 capture through WebUSB.
  Preserve the existing device protocol and acquisition state machine behind an asynchronous USB transport; treat
  browser support, secure-context requirements, permission, interface claiming, and disconnects as capabilities and
  diagnostics rather than providing a synthetic live source.

### Node graph editor

- [graph.editor.socket-renaming] Add generic instance-local socket renaming. Node definitions explicitly mark which input and
  output sockets are renameable; sockets without that capability remain definition-owned. Preserve stable schema IDs and
  runtime port contracts independently from display names, persist user overrides in saved graphs, and provide a way to reset
  a renamed socket to its definition-provided label.

### Graph nodes

- [graph.nodes.measurement-statistics] Add generic measurement and statistics nodes for frequency, duty cycle, pulse width,
  inter-event timing, counts, and histograms.
- [graph.nodes.script-nodes] Add custom script nodes, initially backed by Python, as a plugin/runtime capability with an
  explicit manifest for input/output payload kinds, state schema, parameter defaults, and
  presentation metadata. Run scripts behind a versioned worker boundary with cancellation,
  diagnostics, resource limits, deterministic test fixtures, and an unavailable-platform error;
  do not let scripts access widget state or make the compiler infer contracts from Python code.

### Multi-source timeline

- [graph.timeline.shared-clock-model] Define how several source clocks and trigger positions map onto the shared viewer timeline.
- [graph.timeline.source-grouping] Add graph-level source grouping/alignment metadata and preserve it in saved graphs.

## Refactorings

### Capture provider and host architecture

- [capture.live.provider-unification] Represent file and live sources through one generic capture
  data-provider contract for presentation, readiness, cache/index availability, and data access.
  Providers advertise optional acquisition commands and capabilities, so file sources do not
  pretend to support live acquisition and the application does not branch on file-versus-live
  source kinds to publish artifacts or attach viewer data.
- [capture.live.host-capabilities] Add a host capability that inhibits automatic system sleep while
  acquisition is active. Where inhibition is unavailable, observe suspend/resume and report it as
  a capture-integrity event. Keep the existing generic lifecycle, integrity, and storage contracts
  in `signal_processing`, with no platform conditionals in their consumers.

### Unified native and web data plane

Detailed architecture and capability contracts are documented in
[`docs/WASM_STORAGE_PLATFORM_DESIGN.md`](docs/WASM_STORAGE_PLATFORM_DESIGN.md).

- [platform.data-plane.parity-tests] Add reusable conformance suites for memory, native file, and mmap repositories;
  byte-identical encoded output; exact/presence/boundary queries; growing-prefix visibility; cache planning; source
  preparation; ordered execution; cancellation; corruption; short I/O; and quota exhaustion. Run filesystem-free
  memory tests in every crate build and compile/browser checks for the selected wasm modules.
- [platform.data-plane.browser-persistence] After the shared memory backend is established, add an optional
  worker-owned OPFS artifact repository in `logic_analyzer_platform` with quota reporting, atomic-generation
  publication, eviction recovery, and site-data-loss semantics. Keep OPFS handles and promises in that adapter so
  durable browser caching does not alter store, compiler, or viewer contracts.
- [platform.data-plane.boundary-enforcement] Extend architecture checks to reject target conditionals,
  target-selected modules, `cfg!` target inspection, and target-specific dependencies in every reusable crate except
  `logic_analyzer_platform` and explicitly allowlisted complete file-I/O adapter leaves in
  `logic_analyzer_processing`. Check that application crates remain bootstrap-only, portable node catalogs compile
  from one module tree, and synthetic sources or discard sinks are selected explicitly rather than by target.

### Node-graph extraction

- [graph.extraction.standalone-crate] Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
