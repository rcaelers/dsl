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

- [platform.data-plane.execution.derived-store-encoding] Move derived-word block encoding from the native global
  worker pool to the injected execution contract, preserving bounded encoding work and ordered block publication.
- [platform.data-plane.execution.other-background-work] Inventory and migrate remaining reusable background work
  from direct worker-pool or thread selection to explicit platform execution contracts.
- [platform.data-plane.execution.web-workers] Add an optional Web Worker adapter with serializable work messages,
  cancellation, bounded queues, ordered completion, and explicit unavailable-capability behavior.
- [platform.data-plane.adapter-acquisition-export] Move host capture acquisition, file and browser-handle adapters,
  dialogs, and export destinations into `logic_analyzer_platform`. The compiler, processing nodes, and UI consume
  only their platform-neutral request and capability contracts.
- [platform.data-plane.adapter-embedded-runtime] Move embedded interpreter and runtime-host setup into
  `logic_analyzer_platform` behind a portable execution contract, so concrete node behavior remains target-neutral.
- [platform.data-plane.adapter-usb] Move asynchronous USB transport host adapters into
  `logic_analyzer_platform`. Native USB remains the initial implementation; unavailable browser USB is an explicit
  capability result until a WebUSB adapter is introduced.
- [platform.data-plane.adapter-composition] Complete the `logic_analyzer_platform` service bundle and migrate
  reusable target selection to it. Keep `app_native` and `app_web` as bootstrap-only composition roots, and add
  native and browser composition tests for the injected adapters.
- [platform.data-plane.storage-contracts] Apply the established platform-neutral prepared-byte-source,
  immutable-byte-region, artifact-repository, reader/writer, capability, and error contracts to the existing
  derived and capture stores. Keep paths, mmap, filesystem operations, and browser handles in
  `logic_analyzer_platform`; add native and web repository adapters without changing shared algorithms. This and
  the adapter-crate boundary form the foundation for the remaining work.
- [platform.data-plane.shared-derived-store] Complete one encoded-block decode layer above the shared directory,
  presence index, query, integrity, and decoded-block-cache contracts. Provide native file/mmap and
  platform-independent chunked-memory artifact repositories, keep repository budgets configurable, and remove the
  remaining target-specific range-decode and persistence implementations.
- [platform.data-plane.shared-capture-storage] Run packed raw captures, waveform indexes, growing live repositories,
  and finalized replay through the same artifact and byte-region contracts. Keep native mmap and owned memory as
  interchangeable backings, expose committed generations consistently, and avoid requiring one capture or index to
  fit in one allocation.
- [platform.data-plane.cache-policy] Move cache identity, validation, cached-preview attachment, producer pruning,
  invalidation, publication, pinning, and cleanup policy into the common compiler path. Supply a durable native
  repository and an ephemeral web repository initially; do not replace web cache planning with no-ops merely because
  persistence across reloads is unavailable.
- [platform.data-plane.source-preparation] Make finite-source preparation one capability-driven state machine for
  source resolution, metadata validation, cache lookup/build, index publication, readiness, progress, cancellation,
  and generation replacement. Parsers consume prepared random-access readers instead of `PathBuf`; host acquisition
  remains in `logic_analyzer_platform`, outside the compiler and processing algorithms.
- [platform.data-plane.execution] Define bounded execution semantics for storage and index work: advertised
  parallelism, reader concurrency, backpressure, progress, cancellation, deterministic merge ordering, and failure
  without partial publication. Keep the cooperative implementation portable; put the native worker pool and future
  Web Worker adapter in `logic_analyzer_platform`, selected through injection rather than compiler/runtime target
  modules. Retain explicit serializable work messages as the browser-worker boundary.
- [platform.data-plane.core-source-parity] Remove existing target-selected module trees, target conditionals, and
  target-specific manifest dependencies from `signal_processing`, the compiler, graph nodes, `node_graph`, the
  viewer, reusable widgets, and the UI. Convert application managers, cache backends, source preparation, viewer
  workers, dialogs, preferences, graph services, capture export, decoder execution strategies, registrations, and
  test harnesses to portable code plus injected adapters. In `logic_analyzer_processing`, restrict any remaining
  target selection to the documented temporary file-I/O and USB host-access leaves; keep node schemas, builders,
  parsers, encoders, protocol state machines, and unavailable-capability behavior identical.
- [platform.data-plane.fixed-width-formats] Remove persisted and cross-boundary `usize` values from capture, index,
  cache, manifest, and worker-message formats. Use `u64` offsets and counts with checked conversions only at resident
  slice boundaries, and add tests above the wasm32 addressable range without allocating those ranges so the data
  model remains suitable for future wasm64 builds.
- [platform.data-plane.parity-tests] Add reusable conformance suites for memory, native file, and mmap repositories;
  byte-identical encoded output; exact/presence/boundary queries; growing-prefix visibility; cache planning; source
  preparation; ordered execution; cancellation; corruption; short I/O; and quota exhaustion. Run filesystem-free
  memory tests in every crate build and compile/browser checks for the selected wasm modules.
- [platform.data-plane.browser-persistence] After the shared memory backend is established, add an optional
  worker-owned OPFS artifact repository in `logic_analyzer_platform` with quota reporting, atomic-generation
  publication, eviction recovery, and site-data-loss semantics. Keep OPFS handles and promises in that adapter so
  durable browser caching does not alter store, compiler, or viewer contracts.
- [platform.data-plane.usb-transport] After storage and execution convergence, separate USB discovery/permission from
  control and bulk transport, and make the U3Pro16 protocol depend on the asynchronous transport contract rather than
  the native USB library. Move the native adapter to `logic_analyzer_platform`; this contract enables but does not
  require the lower-priority WebUSB feature.
- [platform.data-plane.boundary-enforcement] Extend architecture checks to reject target conditionals,
  target-selected modules, `cfg!` target inspection, and target-specific dependencies in every reusable crate except
  `logic_analyzer_platform` and explicitly allowlisted complete file-I/O or USB adapter leaves in
  `logic_analyzer_processing`. Check that application crates remain bootstrap-only, portable node catalogs compile
  from one module tree, and synthetic sources or discard sinks are selected explicitly rather than by target.

### Node-graph extraction

- [graph.extraction.standalone-crate] Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
