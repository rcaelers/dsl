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
  protocol and acquisition state machine in `logic_analyzer_device_dslogic` and execute that identical implementation on
  a native background executor or browser worker. Model cancellation without pretending that WebUSB can abort one
  transfer independently; closing a web device may be required to abort its outstanding operations.
- [capture.web.usb-access-preflight] Add a generic asynchronous capture-source access preflight started directly by a
  user gesture. It lets the web host call `requestDevice()` without teaching the UI about USB or U3Pro16, and reports
  unsupported browsers, insecure contexts, denied permission, and unavailable devices as source capabilities and
  user-facing diagnostics.
- [capture.web.usb-worker-session] Establish a worker-owned browser USB session after window permission is granted.
  Platform transfers and owns a generic permitted WebUSB handle and reports neutral transport events. The injected
  U3Pro16 device layer resolves VID/PID, validates runtime identity, selects configuration 1, claims interface 0,
  handles device policy for reconnect/disconnect, and conservatively selects High-Speed acquisition limits unless
  the effective link speed can be established from hardware-validated descriptors.
- [capture.web.usb-fpga-image] Define and implement a lawful browser FPGA-image acquisition policy. The application
  website does not bundle or redistribute `DSLogicU3Pro16.bin`, and users must not have to install DSView merely to
  obtain it. Already-configured devices proceed without an upload. An unconfigured or incompatible device requires
  an independently downloadable vendor-authorized image or an image explicitly selected by the user; if neither is
  available, report that capture cannot configure the FPGA. Persist a user-supplied image only with explicit consent.
- [capture.web.usb-adapter] Implement a device-neutral WebUSB capability in `platform`. Translate
  promises, endpoint numbers, control-request fields, transfer statuses, short transfers, stalls, timeouts,
  cancellation, and disconnects into a neutral USB transport contract. Implement the U3Pro16 transport adaptation
  in its device crate and assemble its source-factory override in `app_web`; platform must not import the U3Pro16
  protocol or graph node. Preserve the existing protocol and capture behavior; never substitute a synthetic source.
- [capture.web.usb-validation] Validate WebUSB with a real U3Pro16 in supported desktop Chromium: first permission,
  permission propagation to the capture worker, interface contention, already-configured and image-required startup,
  finite and streaming capture, trigger headers, sustained throughput, stop/abort, disconnect, reconnect, and browser
  reload. Keep deterministic protocol tests based on a fake asynchronous transport in the processing crate, and keep
  hardware/browser tests explicitly opt-in.

### Node graph editor

- [graph.editor.connection-routing] Add ordered obstacle-avoiding connection drawing without changing
  saved topology or processing behavior. See the [design proposal](writing-block.md) and
  [implementation plan](docs/plans/node_graph_connection_routing.md). One numbered step per branch:
  6. [ ] Complete GPU and full application-frame performance evidence, and address
     remaining cold routing and release/idle frame cost without weakening route constraints.
     Profile the remaining release-frame upper tail and application/GPU costs after conservative
     hit-target move elision; preserve overlap order, pointer capture, and clipped-target Tab order.
     Capture an unoccluded native window and require actual `egui_render` GPU intervals before
     accepting application rendering timings; investigate screenshot completion under occlusion.
     Keep the disjoint reference fixtures free of cold/release fallbacks with the existing work budgets.
     Native/browser CPU baselines, repeated release/idle tails, drag frames, cache-hit measurements,
     and isolated native application UI CPU/cadence baselines (small documents and built-in fan-out scale) are in
     `docs/aspects/performance.md`.
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

Open items carry a priority and severity tag. Priority: P1 = fix first — active structural
inversions; P2 = structural corrections queued behind the P1 items; P3 = planned, often alongside
related work; P4 = later hygiene, or deferred until new evidence promotes it; P5 = blocked on
another item. Severity: high = structural defect that compounds as code is added, medium =
localized boundary violation, low = hygiene or cosmetic. Ordering constraints between items are
noted inline on the dependent item.

This file holds only open work. A completed item is removed once its outcome is documented: the
resulting architecture belongs in `docs/architecture/` or `docs/aspects/`, and performance
evidence, including rejected approaches, belongs in
[Performance Design and Measurement Record](docs/aspects/performance.md).

The change-discipline rules in `AGENTS.md` (one item per branch, relocation not redesign, the
per-step verification commands) apply to every item here. An item large enough to need design
direction gets a working plan under `docs/plans/`, linked from the item and deleted when the item
completes.

### Capture indexing and caching

- [capture.index.acceleration] (P4 · low) Improve finite waveform-index and cache-generation
  throughput. Profiling and CPU-path optimization are complete; source reading and decompression
  are the critical path and the summary kernel is not. See
  [Performance Design and Measurement Record](docs/aspects/performance.md), "Waveform index
  generation". Remaining steps:
  1. [ ] Prototype a batched GPU implementation only for the regular packed digital waveform-summary
     kernel, retaining it only when it beats the optimized CPU baseline while producing bit-exact
     leaf artifacts with the same cancellation, bounded-memory, and progress behavior. The current
     20-worker profiles do not justify starting this prototype: summary work is already off the
     critical path, and GPU dispatch would additionally transfer 1.25–2.73 GB of packed input.
  2. [ ] Preserve platform boundaries: `signal_capture` owns the portable summary kernel, CPU
     fallback, and any adapter from neutral compute operations to capture summaries;
     `platform` owns only device-neutral native/WebGPU access, capability discovery,
     submission, and unavailable-GPU handling. Do not add target conditionals or GPU
     dependencies to portable processing, viewer, compiler, or concrete-node crates. Keep
     decompression, source I/O, protocol decoding, and derived-data caching on their current CPU
     paths unless measurements identify a separate regular, transfer-efficient kernel.

### Optimization backlog (future, priority order)

[Performance Design and Measurement Record](docs/aspects/performance.md) holds the retained
baseline, the reference workloads, and the rejected approaches. Apply the acceptance rule stated
there to every item below: compare both reference captures, exact output and artifact identities,
wall and CPU time, peak memory, cancellation bounds, native/wasm behavior, and concurrent viewer
p99 latency. Do not retain a throughput change that harms foreground response.
Every unchecked item in this section is P4 until new profiling evidence promotes it; the numbered
order below is the internal priority. Use the reproducible regression comparison described in the
[performance record](docs/aspects/performance.md#reproducible-regression-comparisons) before
promoting any item here, so acceptance comparisons remain evidence-based.

1. **Avoid repeated work across cache and graph generations.** This has the highest likely payoff
   because it can remove complete reads, decompressions, decodes, or encodes instead of making an
   already parallel kernel marginally faster.
   - [ ] [capture.index.shared-expanded-block-cache] Attribution confirms cross-consumer reuse; add
     a bounded source-generation-keyed cache for immutable expanded capture blocks. Coalesce concurrent misses,
     preserve channel/block identity and cancellation, and size it from measured reuse distance rather
     than retaining a whole capture. Keep the cache policy generic and concrete DSL archive behavior
     in `logic_analyzer_capture_formats`.
   - [ ] [capture.index.ordered-prefetch] If reads stall the critical path without useful reuse,
     compare bounded block-major lookahead and batched archive entry lookup against the current
     per-worker readers. Do not increase the 12-worker cap or duplicate decompression to hide I/O.
   - [ ] [capture.index.native-mapped-source] Benchmark a platform-owned immutable mapped-file
     source against positional reads for large native captures. Measure page faults, resident memory,
     source-replacement safety, cancellation, and cold/warm behavior; retain positional reads on hosts
     where mapping is unavailable or slower.
   - [ ] [cache.incremental-generation] Key waveform roots and derived lanes by source identity plus
     semantic producer configuration so unchanged artifacts survive graph edits. Recompute only the
     affected downstream subgraph, publish generations atomically, and make every reuse/invalidation
     decision inspectable to the user.
   - [ ] [cache.resumable-generation] Persist enough versioned progress metadata to resume interrupted
     large waveform and derived-cache builds at complete segment boundaries without exposing partial
     generations or weakening corruption checks.
   - [ ] [cache.lazy-reopen] Profile application reopen separately from first generation; lazily map
     directories, presence indexes, and segments, prefetch only the initial viewport, and avoid
     validating or decoding untouched lanes on the UI thread.

2. **Coordinate runtime production, storage, and scheduling end to end.** Local merge, codec,
   ownership-wrapper, segmented-input, and larger-batch probes did not improve the critical path; do
   not repeat them without a representation or scheduling change spanning producer and consumer.
   - [ ] [runtime.performance.critical-path-trace] Add stable per-batch correlation IDs to diagnostic
     metrics and reconstruct source-read, fragment-scan, ordered merge, fan-out, encode, persist, and
     sink spans. Report runnable time versus queue/backpressure wait so optimization targets the wall
     critical path rather than the largest cumulative CPU counter.
   - [ ] [runtime.performance.adaptive-work-allocation] Use the trace to test a bounded allocator for
     fragment scans and derived encoders that shifts shared executor capacity according to queue age,
     ordered-commit blockage, and foreground demand. Preserve deterministic output order and explicit
     per-node limits; never let one dense lane consume every worker.
   - [ ] [runtime.performance.numeric-columnar-batches] Only if copying remains critical, prototype a
     generic numeric-word batch contract with separate value, timestamp, and optional-duration columns
     produced directly by compatible nodes and consumed directly by storage and sinks. Keep payload-
     bearing words on the existing contract, expose capability metadata rather than node-name checks,
     and compare end-to-end against the rejected `Arc<Vec<T>>` and segmented-view approaches.
   - [ ] [runtime.performance.single-pass-derived-blocks] With a columnar or otherwise storage-ready
     producer representation, evaluate one pass that determines boundaries, builds presence summaries,
     and emits encoded columns without first materializing a second `Vec<Word>`. Do not retry the
     previously rejected local eligibility-check fusion on the existing representation.
   - [ ] [runtime.performance.sink-batching] Profile file and CSV sinks independently for formatting,
     buffer growth, copies, and syscalls. Reuse bounded output buffers or vectored writes only where
     sink work is on the critical path; retain filename-window ordering and injected storage APIs.
   - [ ] [runtime.performance.backpressure-tuning] Record queue occupancy and producer blocking under
     dense and sparse captures, then tune bounded capacities as a set. Reject settings that merely
     trade lower wall time for excessive RSS, cancellation latency, or viewer contention.

3. **Improve interactive responsiveness and perceived latency.** Current viewer measurements are
   already below the 8 ms frame budget, so prioritize avoiding redundant work and stale results over
   raw throughput.
   - Start with the existing debounced live-sync task below: replace fixed-
     interval semantic graph polling with an event-driven dirty revision and true debounce, lower
     immutable revisions off the UI thread, and discard stale results before runtime application.
   - [ ] [viewer.query-generation-cache] Cache lane queries by immutable lane generation, viewport,
     resolution, and presentation contract. Reuse overlapping results, invalidate explicitly, and
     never infer protocol behavior from lane or port names.
   - [ ] [viewer.query-prefetch] After measuring pan/zoom access patterns, prefetch one bounded adjacent
     time range at the current resolution and cancel it immediately when the viewport generation
     changes. Foreground queries always outrank prefetch and cache generation.
   - [ ] [viewer.progressive-detail] Render a coarse presence/summary result first, then replace it with
     exact decoded detail asynchronously when the zoom level requires it. Preserve stable hit testing
     and avoid visual changes caused by completion order.
   - [ ] [ui.frame-budget-scheduler] Add diagnostic frame-budget accounting for graph lowering, cache
     publication, lane queries, uploads, and painting. Defer bounded background work when predicted
     frame cost would cross 8 ms, without changing processing correctness or hiding stalled work.
   - [ ] [ui.graph-rebuild-minimization] Cache layout, tessellation, and node-widget state by semantic
     and visual revisions so progress updates do not rebuild unchanged graph or lane geometry.

4. **Use the GPU only where data is regular and reuse amortizes transfer.** GPU acceleration remains
   conditional; ZIP inflation, variable-length derived encoding, generic graph execution, and small
   protocol streams are not current GPU candidates.
   - [ ] [viewer.gpu-waveform-rendering] Profile CPU tessellation and upload cost at high lane counts
     and dense zoom levels. If material, render generic waveform/presence primitives from compact
     instance buffers with bounded incremental uploads and an equivalent CPU fallback.
   - [ ] [viewer.gpu-decoded-lanes] Define protocol-neutral glyph/span instance metadata for decoded
     lanes and benchmark GPU instancing only after query and tessellation attribution shows a frame-
     time bottleneck. Concrete nodes provide presentation metadata; the viewer never branches on
     protocol names or payload values.
   - [ ] [capture.index.gpu-summary] Keep the existing packed digital summary prototype deferred until
     summary work is again on the cold-build critical path. Batch enough already-resident packed data
     to amortize dispatch, require bit-exact artifacts, and keep the portable CPU implementation.
   - [ ] [runtime.gpu-packed-scan] Reconsider packed parallel scanning only if future profiles show one
     regular scan kernel dominating wall time and its inputs can remain GPU-resident across several
     operations. Include transfer, synchronization, cancellation, and result-ordering costs; do not
     add GPU branches to generic runtime/compiler infrastructure.
   - [ ] [platform.gpu-capability] If any GPU prototype wins, define a device-neutral compute
     capability below its consumers and implement native/WebGPU access in
     `platform`. Keep kernel/domain adaptation and portable CPU fallbacks in their
     owning core crates, inject the neutral capability at composition roots, and expose
     availability/fallback diagnostics. Never make cache identity depend on the selected device.

- [performance.telemetry-overhead] (P4 · low) Measure profiling counters disabled and enabled; sample or aggregate
  hot-path metrics so observability cannot become the bottleneck it is intended to diagnose.
- [performance.web-baselines] (P4 · medium) Establish equivalent browser-worker baselines for waveform generation,
  derived caching, graph edits, and viewer input latency using the same artifact identities and
  bounded-memory rules. Native improvements are not assumed to help wasm without measurements.

### Graph-node editor separation

- [graph.nodes.editor-split] (P3 · medium) Split editor definitions out of
  `logic_analyzer_graph_nodes` so the concrete node bundle is headless. 54 of its 170 files
  import `node_graph::api` (`NodeDef`, `SocketDef`, `NodeTypeRegistry`, inline controls), so the
  bundle — and every composition that builds a registry snapshot, including worker-side lowering
  and headless runs — compiles against the egui widget crate. The seam already exists:
  `logic_analyzer_graph_editor_registry` binds stable graph-feature IDs to editor definitions,
  and headless graph crates do not depend on it. Move the `NodeDef`/editor faces into a sibling
  bundle crate (working name `logic-analyzer-graph-node-editors`) registered through the editor
  registry under the same stable feature IDs — the same split already executed for the registry
  itself and for timeline markers. Apply the identical split to the example plugin's two
  `node_graph::api` imports so a headless-only plugin becomes possible. Acceptance:
  `logic_analyzer_graph_nodes` and `example-plugin` manifests have no `node-graph` dependency,
  locked by an edge assertion in `tests/architecture_dependencies_tests.rs`; saved graphs, stable
  IDs, and registration behavior unchanged.

### Node-graph extraction

- [graph.extraction.standalone-crate] (P5 · low)
  Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.

### Application state decomposition

- [ui.app.behavior-migration] (P3 · low) Move the remaining `App` behavior onto the owner types
  the state decomposition created. `app.rs` is still 4,392 lines because the methods stayed
  behind when the fields moved: `sync_run` and `start_run` belong on the `graph_run_lifecycle`
  owner, `sync_capture_analysis` and `show_capture_controls` on `capture_analysis_lifecycle` or a
  capture panel module, and `synchronize_timeline_marker_references` on
  `timeline_marker_bindings`. Mechanical moves guided by the borrow checker, per the
  ui.app.decomposition method already applied to the fields; cross-owner needs become explicit
  method arguments, never `&mut App`. Target: `app.rs` under about 1,500 lines of composition and
  panel dispatch.
- [panel-layout.extraction.standalone-crate] (P4 · low)
  Prepare `panel-layout` for independent publication: replace workspace-inherited package and
  dependency metadata, move its documentation and examples with the crate, add standalone CI, and
  verify that its persisted layout, area, panel, and view contracts remain application-neutral.

`logic_analyzer_ui` is now the largest crate (about 19,700 lines, 21 workspace dependencies).
Its content is legitimately application composition plus panels, and the module-ownership tests
police it internally, so no split is scheduled. If it keeps absorbing features, the panels
(`memory_panel`, `decoder_panel`, `preferences`, `about`) are the extraction seam; revisit when a
new panel family lands.

### Module ownership

- [modules.large-leaf-ownership] (P4 · low) Bring the remaining large leaves under the
  module-ownership rule: `parallel_decoder/decoder.rs` (2,880 lines),
  `derived_word_store/store.rs` (2,399), the DSLogic `driver.rs` (2,398), and
  `signal_runtime/manager.rs` (2,380) with `cooperative_manager.rs` (1,572). Documentation first:
  answer the four ownership questions (data/invariants, facade, permitted dependencies,
  exclusions) in each module doc. Split physically only where it clarifies ownership — these are
  hot-path files with measured history, so any split must clear the acceptance rule in
  [Performance Design and Measurement Record](docs/aspects/performance.md); do not decompose the
  decoder loop for aesthetics.

### Naming

- [naming.platform-prefix] (P4 · low) Resolve the double meaning of the `platform` prefix:
  `platform_artifacts` and `platform_runtime` are portable contracts nearly everything depends
  on, while `platform` is the one target-selected adapter crate — a reader of the crate list will
  guess wrong about at least one of them. Renaming the adapter crate (for example
  `host-adapters`) is the smaller diff since it has few consumers; renaming the contract crates
  is the alternative. Decide once and do it soon or not at all — the churn only grows with every
  new consumer.
