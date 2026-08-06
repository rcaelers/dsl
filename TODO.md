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

Every P1–P3 item has design and implementation direction — current wiring, target shape,
ordered steps, and acceptance checks — in
[P1/P2 Refactoring Directions](docs/plans/refactoring_p1_p2.md) and
[P3 Refactoring Directions](docs/plans/refactoring_p3.md). Read the item's section, and the
ground rules at the top of the P1/P2 document, before starting one of these items.
The dependency graph is consistent with the priority ordering: ordering constraints are stated on
the affected items, and no P3 item gates a P2.

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
order below is the internal priority. Build [performance.regression-harness] before promoting any
item here, so acceptance comparisons stop being ad-hoc.

1. **Avoid repeated work across cache and graph generations.** This has the highest likely payoff
   because it can remove complete reads, decompressions, decodes, or encodes instead of making an
   already parallel kernel marginally faster.
   - [ ] [capture.index.archive-work-attribution] Count compressed entries opened, compressed and
     expanded bytes, source ranges reread, decompressions, cache hits, and wait time per source
     generation across waveform-index construction, runtime block delivery, and concurrent viewers.
     Prove duplicate work before changing archive ownership or scheduling.
   - [ ] [capture.index.shared-expanded-block-cache] If attribution confirms reuse, add a bounded
     source-generation-keyed cache for immutable expanded capture blocks. Coalesce concurrent misses,
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

- [performance.regression-harness] (P3 · medium) Turn the existing capture benchmarks into an opt-in reproducible
  comparison report with warmup policy, alternating A/B order, median and spread, exact identity
  checks, peak RSS, CPU, viewer percentiles, and retained baseline metadata. Keep large captures out
  of ordinary unit tests, but make it difficult to accept noisy or microbenchmark-only improvements.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#performance-regression-harness).
- [performance.telemetry-overhead] (P4 · low) Measure profiling counters disabled and enabled; sample or aggregate
  hot-path metrics so observability cannot become the bottleneck it is intended to diagnose.
- [performance.web-baselines] (P4 · medium) Establish equivalent browser-worker baselines for waveform generation,
  derived caching, graph edits, and viewer input latency using the same artifact identities and
  bounded-memory rules. Native improvements are not assumed to help wasm without measurements.

- [graph.execution.debounced-live-sync] (P3 · medium) Replace fixed-interval semantic graph polling with an
  event-driven dirty revision and a true debounce: reset the quiet-period timer after every
  processing-relevant edit, lower only the latest immutable graph revision after the quiet period,
  and discard stale results when a newer revision exists. Perform lowering and edit-plan
  preparation away from the UI thread, keep runtime application ordered through its control
  boundary, and leave periodic progress reporting independent from graph synchronization.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#graph-execution-debounced-live-sync).

### Capture provider and host architecture

- [capture.live.provider-unification] (P3 · medium) Represent file and live sources through one generic capture
  data-provider contract for presentation, readiness, cache/index availability, and data access.
  Providers advertise optional acquisition commands and capabilities, so file sources do not
  pretend to support live acquisition and the application does not branch on file-versus-live
  source kinds to publish artifacts or attach viewer data.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#capture-live-provider-unification).
- [capture.live.host-capabilities] (P4 · low) Add a host capability that inhibits automatic system sleep while
  acquisition is active. Where inhibition is unavailable, observe suspend/resume and report it as
  a capture-integrity event. Keep the existing generic lifecycle, integrity, and storage contracts
  in `signal_capture_session`, with no platform conditionals in their consumers.

### Node-graph extraction

- [graph.document-model-extraction] (P2 · medium) Extract the graph document model out of the
  `node-graph` widget crate. `Socket` currently combines semantic identity with editor
  presentation (`egui::Color32`, labels, shape, visibility, and controls), while graph capability
  contracts accept the complete type. That makes presentation state visible to compiler-facing
  semantics and prevents a genuinely headless graph tier. Introduce a neutral semantic socket
  reference containing only stable schema/member identity and direction, and move the persisted
  document records (`GraphState`, nodes, sockets, connections, frames, neutral positions and
  presentation values) into a small document crate consumed by both the widget and graph tier.
  The widget maps those neutral records to egui types. Remove `node-graph` from plan, runtime,
  capabilities, orchestration, and web-worker execution; then remove it from compiler and registry
  once editor registration is separated from runtime capability registration. Preserve serde
  shape and saved-graph migrations explicitly, and assert every resulting manifest boundary.
  The current direction's identities-only slice is insufficient because capabilities consume the
  full UI-bearing `Socket`; revise it before implementation:
  [refactoring_p1_p2.md](docs/plans/refactoring_p1_p2.md#graph-document-model-extraction).
- [graph.extraction.standalone-crate] (P5 · low — blocked by [graph.document-model-extraction])
  Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.

### Error contracts

- [errors.typed-boundaries] (P3 · medium) Replace `Result<_, String>` on cross-crate contracts with owned error
  types so failures carry a responsibility and callers can classify them. Roughly 360 signatures
  use a string error today; `platform`, `logic_analyzer_ui`, `platform_runtime`,
  and `signal_runtime` hold many of them. Work outward from the lowest owner so downstream crates
  inherit typed failures instead of re-wrapping strings.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#errors-typed-boundaries).
  1. [ ] Give `platform_runtime` typed executor, task, worker-message, kernel-registration, and
     queue errors next to `WorkerMessageError`.
  2. [ ] Complete the `signal_runtime` error surface next to `ConnectionError`, `PortError`, and
     `WorkError`, covering manager and pipeline-supervision failures.
  3. [ ] Type the graph capability and host-override contracts, including
     `SigrokCatalogScanner`, `SigrokDecoderRuntime::discover`, and `SigrokDecoderRuntime::create`,
     so the domain adapter reports discovery, transport, and configuration failures distinctly;
     neutral platform capabilities expose their own mechanism-level errors without importing
     these domain contracts.
  4. [ ] Type source preparation and run diagnostics so `SourcePreparationUpdate::Failed` and the
     UI's run-message path stop matching on message text.
  5. [ ] Keep display strings at the presentation boundary only; generic crates map a concrete
     format or transport failure into their own variant rather than formatting it early.

### Composition and host wiring

- [derived.cache.global-state] (P2 · high) Give the decoded-block cache an owned handle instead of the
  process-global `configure_decoded_block_cache`, `decoded_block_cache_stats`, and `clear_cache`
  entry points in `signal_derived`. The memory panel and cache commands then act on a service the
  application owns rather than on ambient state, allowing multiple application instances and
  isolated concurrent tests.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#derived-cache-global-state).

### Application state decomposition

- [ui.app.decomposition] (P3 · high) Split `logic_analyzer_ui::App`. One struct with about fifty fields across
  4,390 lines owns run lifecycle, capture lifecycle, trigger configuration, timeline markers,
  presentation catalogs, panel state, and notifications, so no field's invariants are stated
  anywhere. Extract owned types for the graph-run lifecycle, the capture-analysis lifecycle, the
  presentation catalogs, and the timeline-marker bindings, each holding its own invariants and
  exposing methods rather than fields. `App` retains composition and frame dispatch. Resolve
  [ui.graph-service.port-shape] first so the graph-run lifecycle is extracted against the port's
  final shape, and state the [ui.boundaries.module-ownership] rules before or alongside so the
  decomposition follows written rules rather than defining them implicitly.
  Direction, including the field-to-owner grouping:
  [refactoring_p3.md](docs/plans/refactoring_p3.md#ui-app-decomposition).
- [ui.capture.coordinator-decomposition] (P3 · high) Split `live_capture/coordinator.rs` along the same lines.
  Its 2,867 lines mix acquisition commands, event polling, storage publication, and status
  presentation; the acquisition state machine and the presentation projection are separate owners.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#ui-capture-coordinator-decomposition).
- [ui.boundaries.module-ownership] (P3 · medium) Extend the owner-boundary rules in
  `docs/aspects/responsibility_visibility.md` to substantial modules inside a crate. Crate-level
  ownership statements currently stop at the crate wall, which is why the largest single-owner
  violations are invisible to the architecture documentation.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#ui-boundaries-module-ownership).
- [readability.large-module-decomposition] (P3 · medium) Decompose oversized implementation leaves
  whose crate responsibility is sound but whose internal ownership is difficult to read.
  `widgets/panel_layout/src/lib.rs` combines persisted state, geometry, layout algorithms, action
  reduction, rendering, pointer interaction, and tests in more than 3,500 lines;
  `node_graph`'s graph interaction leaf and the trigger editor have similar navigation costs.
  Extract cohesive private leaf modules behind the existing owner facade before considering new
  crates. Keep behavior and public paths stable, and use the module-ownership rules above to name
  each leaf by the behavior it owns.
- [panel-layout.extraction.standalone-crate] (P5 · low — blocked by [readability.large-module-decomposition])
  Prepare `panel-layout` for independent publication: replace workspace-inherited package and
  dependency metadata, move its documentation and examples with the crate, add standalone CI, and
  verify that its persisted layout, area, panel, and view contracts remain application-neutral.

### Crate boundary corrections

- [session.domain-relocation] (P3 · medium) Purge logic-analyzer vocabulary from the generic session tier.
  `signal_capture_session` publishes 129 items and a public `logic_analyzer` module, and the
  trigger program, trigger schema, and `SimpleTriggerCondition` types it owns are domain concepts
  that generic acquisition does not need. Move trigger data into a small logic-analyzer trigger
  domain and move source/driver contracts to their concrete acquisition owners; do not retain
  compatibility re-exports through the generic crate.
  The trigger vocabulary is the highest-leverage cluster: `logic_analyzer_viewer`
  (`simple_trigger.rs`) and `logic_analyzer_graph_compiler` also import it from the session
  crate, so three consumers currently reach into the generic tier for domain types.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#session-domain-relocation).
- [session.facade-glob] (P4 · low) Replace wildcard facade exports with explicit supported
  lists, as the facade rule requires. Current crate-root examples are
  `signal_capture_session::live_capture_store::*`, `logic_analyzer_graph_plan::plan::*`, and
  `logic_analyzer_graph_runtime::runtime::*`. Treat the lists as API contracts and remove duplicate
  public paths while doing [node-graph.single-import-path].
- [derived.payload.builtin-registration] (P3 · medium) Register built-in derived payload kinds through
  `PayloadRegistry` like every other payload and purge product vocabulary from `signal_derived`.
  The crate currently has both an open registry and a closed built-in set
  (`digital_payload_adapter`, `word_payload_adapter`, `trigger_payload_adapter`,
  `TriggerLaneSnapshot`, `ProtocolPacket`). Generic retained-value, query, and storage contracts
  stay in `signal_derived`; trigger/protocol semantics and their registrations move to the
  corresponding logic-analyzer feature owners.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#derived-payload-builtin-registration).
- [ui.graph-service.port-shape] (P3 · medium) Resolve the `GraphService` port. Its contract is typed in
  `ProcessingGraph`, `GraphRunContext`, `ApplySummary`, `SourceReadinessRegistry`, and
  `ProcessingGraphError`, so the UI manifest still depends on the compiler, runtime, plan,
  orchestration, registry, and capability crates. Either narrow the port to UI-shaped types and
  drop those dependencies, or remove the indirection and document that the UI owns graph
  execution. The present shape costs a trait and its adapters without reducing coupling.
  Review recommendation: remove the trait and document that the UI owns graph execution.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#ui-graph-service-port-shape).
- [node-graph.category-ordering] (P4 · low) Replace the `category.label == "External Sigrok"` sort key in
  `node_graph`'s add-menu construction with an ordering value supplied by the category metadata.
  It is the one place a generic widget branches on a protocol name.
- [node-graph.single-import-path] (P4 · low — after [graph.document-model-extraction], which reshapes the same crate root) Stop re-exporting the whole `api` namespace from the `node_graph`
  crate root. Both `node_graph::NodeDef` and `node_graph::api::NodeDef` resolve today, so the
  documented split between the compiler-facing namespace and the editor facade is unenforced.
  The crate root additionally re-exports `model::{GraphState, NodeId, …}` and `runtime::{…}`
  directly, so the same types resolve through three paths, not two.

### Enforcement and documentation

- [tests.architecture-structural] (P2 · medium) Replace the source-text architecture tests with structural
  checks. About 1,700 lines across the workspace `include_str!` a sibling file and assert on
  `.contains("…")`, so they break on a rename or a reformat, pass when the string appears in a
  comment, and prove nothing about the compiled contract. Enforce dependency direction from the
  manifests and enforce capability rules by constructing a registry and asserting on the resulting
  descriptors. Manifest-based checks would have caught platform's UI, graph, and processing edges
  and the widget dependency in the graph execution tier; the string tests did not. Assert that
  platform depends only on neutral host contracts and generic infrastructure, and that compiler
  and registry also lose the widget dependency after editor registration is separated. Do not
  mark future boundary assertions ignored: the repository forbids ignored tests. Use an explicit
  temporary violation allowlist keyed by TODO ID, fail on every unlisted edge, and delete each
  exception in the refactoring that removes it. Land the manifest checks together with the P1
  composition items so restored boundaries are locked in as they are established.
  A workspace `cargo metadata` test now rejects every platform dependency on a Logic Conduit
  domain crate; continue converting the remaining source-text checks and add the other dependency
  rules listed in the direction.
  Direction, including the forbidden-edge list:
  [refactoring_p1_p2.md](docs/plans/refactoring_p1_p2.md#tests-architecture-structural).
- [docs.drift-correction] (P3 · medium) Correct the design statements the code no longer satisfies: `AGENTS.md`
  still describes a `signal_processing` crate that no longer exists. `AGENTS.md` also assigns execution and saved-document
  synchronization too broadly to the compiler; `responsibility_visibility.md` both permits and
  forbids `pub(super)`; its UI-owned-port statement must be reconciled with the chosen
  domain-neutral platform boundary. The P1/P2 direction currently contradicts this TODO by keeping
  platform→processing, proposing an omnibus host-ports crate, limiting graph extraction to
  identities, recommending domain vocabulary in `signal_*`, counting five processing binaries,
  and suggesting ignored architecture tests. Revise that direction before implementation.
  Normative documents are load-bearing, so each correction either describes current behavior or
  is paired with the item that restores the stated behavior; target architecture belongs in plans,
  not in present-tense design documents.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#docs-drift-correction).
- [docs.owner-local-detail] (P4 · low) Keep design detail with its owning crate. The viewer document
  currently describes capture file formats, indexes, and mipmaps owned by capture/processing and
  contains stale source paths. Move authoritative format and indexing descriptions to their owners
  and leave the viewer document describing only how it consumes those contracts.
- [docs.ownership-statements] (P4 · low) State crate ownership positively. Large parts of
  `crate_responsibility.md` define a crate by what it excludes; one sentence naming what it owns
  and the type it hands to the next layer carries more. Keep an exclusion only where the boundary
  is genuinely surprising.
- [docs.index-deduplication] (P4 · low) Reduce `docs/INDEX.md` to one entry per document. Most crates appear
  in three lists, so the index has become a table of contents for itself.
- [naming.implementation-files] (P3 · low) Rename the repeated `implementation.rs` leaves after the behavior
  they hold. The name carries no information, collides in editor tabs and search results, and
  appears 46 times across the node crates. The capability decomposition made this more urgent:
  `logic_analyzer_graph_nodes` averages about 100 lines per file, so file names now do the
  navigation work that file contents used to.
  Direction: [refactoring_p3.md](docs/plans/refactoring_p3.md#naming-implementation-files).
