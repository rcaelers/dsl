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

- [capture.web.file-export] Let web users export captures and generated files through an explicit destination acquired
  by a user gesture. Keep downloads separate from internal cache publication and report unsupported or lost
  permissions without changing processing-node behavior.
- [capture.web.usb-async-transport] Replace the U3Pro16 transport's blocking open, control-transfer, bulk-transfer,
  timeout, and queued-read boundary with a portable asynchronous or explicitly pollable contract. Keep the device
  protocol and acquisition state machine in `logic_analyzer_processing` and execute that identical implementation on
  a native background executor or browser worker. Model cancellation without pretending that WebUSB can abort one
  transfer independently; closing a web device may be required to abort its outstanding operations.
- [capture.web.usb-access-preflight] Add a generic asynchronous capture-source access preflight started directly by a
  user gesture. It lets the web host call `requestDevice()` without teaching the UI about USB or U3Pro16, and reports
  unsupported browsers, insecure contexts, denied permission, and unavailable devices as source capabilities and
  user-facing diagnostics.
- [capture.web.usb-worker-session] Establish a worker-owned browser USB session after window permission is granted.
  Resolve the permitted U3Pro16 by VID/PID, validate its runtime identity, select configuration 1, claim interface 0,
  handle reconnect/disconnect, and conservatively select High-Speed acquisition limits unless the effective link
  speed can be established from hardware-validated descriptors.
- [capture.web.usb-fpga-image] Define and implement a lawful browser FPGA-image acquisition policy. The application
  website does not bundle or redistribute `DSLogicU3Pro16.bin`, and users must not have to install DSView merely to
  obtain it. Already-configured devices proceed without an upload. An unconfigured or incompatible device requires
  an independently downloadable vendor-authorized image or an image explicitly selected by the user; if neither is
  available, report that capture cannot configure the FPGA. Persist a user-supplied image only with explicit consent.
- [capture.web.usb-adapter] Implement the WebUSB U3Pro16 transport and source-factory override in
  `logic_analyzer_platform`. Translate WebUSB promises, endpoint numbers, control-request fields, transfer statuses,
  short transfers, stalls, timeouts, cancellation, and disconnects into the portable transport contract. Preserve the
  existing protocol and capture behavior; never substitute a synthetic live source.
- [capture.web.usb-validation] Validate WebUSB with a real U3Pro16 in supported desktop Chromium: first permission,
  permission propagation to the capture worker, interface contention, already-configured and image-required startup,
  finite and streaming capture, trigger headers, sustained throughput, stop/abort, disconnect, reconnect, and browser
  reload. Keep deterministic protocol tests based on a fake asynchronous transport in the processing crate, and keep
  hardware/browser tests explicitly opt-in.

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

### Capture indexing and caching

- [capture.index.acceleration] Improve finite waveform-index and cache-generation throughput in this
  order:
  1. [x] Profile representative captures, reporting separate read/decompression, packed-block
     handoff/copy, summary-kernel, and artifact-publication timings. The initial `scan.dsl` cold
     build attributes 1.61 s of its 1.66 s wall time to source reading/decompression; summary work
     consumes 0.38 cumulative CPU-seconds, while copying and in-memory artifact publication consume
     about 22 ms and 15 ms respectively. After CPU optimization, a second 2.73 GB packed-input
     profile completes in 1.71 s with 7.66 cumulative worker-seconds in reads and 0.73 in summaries,
     confirming source reading/decompression remains the critical path.
  2. [x] Optimize the CPU path first: remove avoidable packed-block copies and keep bounded source
     read, CPU summary, and artifact-write work pipelined through the existing host executor. Local
     workers retain shared `BlockData` backing, each bounded worker owns one source reader, and the
     coordinator publishes completed leaves in per-channel order. Five post-change `scan.dsl` runs
     have a 0.76 s median, versus 1.66–1.69 s before the change, for an approximately 2.2× median
     speedup with zero handoff-copy time.
  3. [ ] Prototype a batched GPU implementation only for the regular packed digital waveform-summary
     kernel, retaining it only when it beats the optimized CPU baseline while producing bit-exact
     leaf artifacts with the same cancellation, bounded-memory, and progress behavior. The current
     20-worker profiles do not justify starting this prototype: summary work is already off the
     critical path, and GPU dispatch would additionally transfer 1.25–2.73 GB of packed input.
  4. [ ] Preserve platform boundaries: `signal_processing` owns only a portable capability contract
     and CPU fallback; `logic_analyzer_platform` owns native and WebGPU adapters, capability
     discovery, batching, and unavailable-GPU handling. Do not add target conditionals or GPU
     dependencies to portable processing, viewer, compiler, or concrete-node crates. Keep
     decompression, source I/O, protocol decoding, and derived-data caching on their current CPU
     paths unless measurements identify a separate regular, transfer-efficient kernel.

### Derived-data storage

- [derived.storage.segmented-artifacts] Replace one-file-per-derived-block publication with a
  bounded number of large immutable segment artifacts. Encode blocks concurrently, append their
  ordered bytes into segment-sized writable mappings or buffered regions, and publish only complete
  segments plus the final index/manifest generation. Native mappings rely on ordinary OS page-cache
  writeback rather than a durability barrier per block; web storage uses the same segment/index
  model over its injected repository. Preserve atomic generation visibility, cancellation cleanup,
  exact range queries, cache portability, and corruption validation. Use `logic-conduit run
  graphs/spi_controlled_decode.json --json` as the end-to-end acceptance benchmark and keep artifact
  count, bytes, execution time, CPU utilization, and final-publication latency visible in its report.

### Graph execution

- [graph.execution.debounced-live-sync] Replace fixed-interval semantic graph polling with an
  event-driven dirty revision and a true debounce: reset the quiet-period timer after every
  processing-relevant edit, lower only the latest immutable graph revision after the quiet period,
  and discard stale results when a newer revision exists. Perform lowering and edit-plan
  preparation away from the UI thread, keep runtime application ordered through its control
  boundary, and leave periodic progress reporting independent from graph synchronization.

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

### Node-graph extraction

- [graph.extraction.standalone-crate] Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
