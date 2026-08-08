# P3 Refactoring Directions

Companion to [P1/P2 Refactoring Directions](refactoring_p1_p2.md); the ground rules there (read
`AGENTS.md`, one item per PR, relocation not redesign, test commands, updating string
architecture tests) apply to every item here and are not repeated. [`TODO.md`](../../TODO.md)
owns priorities and ordering constraints; delete each section when its item completes and the
outcome is documented.

P3 items are planned work, often alongside related changes. The module-ownership rules in
[`responsibility_visibility.md`](../aspects/responsibility_visibility.md#module-ownership) guide
the remaining UI decompositions.

## errors.typed-boundaries (P3 · medium) {#errors-typed-boundaries}

The remaining string-error surfaces are concentrated in platform, UI, graph nodes, and concrete
processing. `platform_runtime` owns typed work-executor, worker-kernel, queue, message, and
terminal-failure contracts. `signal_runtime` owns `PortError`, `ConnectionError`, `PipelineError`,
and `WorkError`; supervised `NodeFailure` values retain their `WorkError`. Extend those
owner-specific surfaces rather than replacing them with an umbrella error.
`logic_analyzer_protocol_decoders` owns the typed Sigrok catalog and decoder-runtime errors;
host adapters classify catalog discovery, decoder discovery, invalid configuration, and execution
transport failures before the graph materializer maps them into its generic build diagnostic.
`logic_analyzer_graph_runtime` owns `SourcePreparationError`, including discovery, metadata, index,
cancellation, executor, and worker-protocol causes. It also owns `DerivedCacheError`; synchronous
and asynchronous cache administration retain the generic derived-store or host-executor cause until
UI presentation or graph-worker serialization. `signal_capture` owns separate capture-worker codec,
bounded-client, serializable transport, and terminal-operation failures. Capture-worker source
preparation preserves those lower typed causes in `SourcePreparationError`. Its generic host-backed
index query port classifies submission, execution, cancellation, disconnection, and invalid-update
failures without exposing the worker protocol; source-bearing variants preserve the concrete host
or worker cause through `CaptureIndexProxy`. `signal_capture_session` owns
`CaptureSourceMetadataError`; lazy presentation and channel discovery distinguish source access
from metadata decoding, while live acquisition construction has its own category. Prepared-file
and device adapters retain their concrete I/O, parser, and acquisition causes through that facade.
The generic graph feature wraps metadata inspection in `CaptureSourceFeatureError`, and the neutral
graph-plan exchange contract carries feature, source-identity encoding, and multiple-source
selection failures through `CapturePresentationDiscoveryError`. Compiler discovery and runtime
source preparation therefore share the typed result without depending on one another. Index
metadata inspection and index opening or construction retain `signal_capture::Error` in distinct
`SourcePreparationError` variants. Executor admission and loss retain
`platform_runtime::WorkExecutorError`, and graph runtime classifies an unexpected capture-worker
response with `SourcePreparationProtocolError`.
The reusable host picker classifies file reads, capacity limits, imports, and missing dropped-file
contents through `platform::FilePickerError`. Application composition preserves that error as the
source of the independent widget-owned `node_graph::FileDialogError`; the inline file control is
the presentation boundary that formats it.
Reusable document mechanisms retain filesystem and browser-session causes in
`platform::DocumentError`. Native and browser application roots map that source into the UI-owned
`GraphDocumentError`, which distinguishes host reads, JSON decoding, JSON encoding, and host writes
until toast or headless presentation.
Browser output activation distinguishes queue expiry from payload, URL, document, link, and cleanup
stages through `platform::DownloadError`. Web composition retains that error as the source of the
UI-owned `OutputDownloadError`, and the application toast is the presentation boundary.
Worker-adapter construction retains portable queue configuration and native thread-start causes,
and classifies browser bootstrap-payload, URL, worker-start, and cleanup failures through
`platform::WorkerAdapterError`. Native composition propagates the error to application startup;
web composition formats it only when describing why the portable cooperative fallback was selected.
Browser artifact-repository opening distinguishes invalid roots, host persistence-worker stages,
unavailable durable storage, invalid initialization responses, and session hydration through
`platform::ArtifactRepositoryOpenError`. Hydration retains the portable `RepositoryError`; web
composition formats the typed failure only when reporting why it selected the memory repository.
Generic native USB opening distinguishes a complete selector miss from context, enumeration,
descriptor, device, identity, configuration, and interface failures through
`platform::UsbDeviceOpenError`. Host failures retain their `rusb::Error`. The existing
driver-neutral acquisition transport variant remains string-only, so the native device adapter is
the current formatting boundary for this source.

`logic_analyzer_graph_orchestration` owns
separate graph-worker codec, bounded-client, and serializable transport failures. The browser host
retains those categories through disconnect and terminal messages; the UI graph-run adapter formats
them only when projecting its presentation-facing diagnostic. `logic_analyzer_capture_export`
likewise preserves export metadata, capture consistency, capture-store, destination, archive, and
cancellation causes.
Its service retains capture-store and executor sources, worker loss, and the typed exporter failure;
UI policy therefore matches a cancellation variant rather than display text.

**How to type an error here** (`thiserror` is already a workspace dependency):

- One enum per *facade*, not per crate and not per function. Variants describe what failed in
  the owner's vocabulary; a wrapped lower-level error rides in a variant field.
- Convert at ownership boundaries: a crate maps a dependency's error into its own variant
  (`#[from]` only when the dependency type is itself part of the crate's contract). Never
  `format!` a message and pass it up — formatting happens once, at the presentation boundary.
- `Result<_, String>` in *tests* is fine; do not churn test code.

**Order (work outward from the lowest owner, per the TODO item):**

1. Make the driver-neutral acquisition transport error source-bearing and retain
   `platform::UsbDeviceOpenError` through native DSLogic device construction.
2. Continue through the remaining platform and UI facades: most occurrences collapse into carrying
   the now-typed lower errors; only genuinely UI-owned failures need new variants.

Expect this to span many small PRs; each facade conversion is independently landable.

## graph.execution.debounced-live-sync (P3 · medium) {#graph-execution-debounced-live-sync}

**Current state.** `app.rs:2772` — `const SYNC_INTERVAL_S: f64 = 0.5`: every 0.5 s the UI thread
computes `self.node_graph.graph().semantic_snapshot()` and compares it against
`cached_preview_graph` (`app.rs:2784`), then refreshes capture availability, trigger
configuration, and sampling-overlay candidates. A parallel 0.5 s epoch poll exists at
`app.rs:2421` (`EPOCH_SYNC_INTERVAL_S`). Cost is paid when idle; latency is paid when editing.

**Direction.**

1. Add a monotonically increasing *semantic revision* to the graph document, bumped only by
   processing-relevant edits (node/connection/state changes — not node positions or panel
   state). Home: `node_graph_document`, which owns document-local semantic state.
2. Replace the interval comparison with a true debounce: on each frame, if
   `document_revision != last_lowered_revision` and `now - last_edit_time >= quiet_period`
   (start at 250 ms), take one immutable snapshot and submit it for lowering; reset the timer on
   every relevant edit. An unchanged graph costs one integer compare per frame, not a snapshot.
3. Perform lowering/edit-plan preparation off the UI thread. The machinery exists: the
   orchestration worker client already lowers on a worker, and `worker_operation_executor` is
   available natively. Tag each submission with its revision; when a result arrives, apply it
   only if its revision is still current — otherwise drop it.
4. Keep run-progress pumping (`run.pump_for(…)` at `app.rs:2807`) on its existing cadence;
   progress reporting is explicitly independent of graph synchronization.
5. Measure before/after with the concurrent-viewer methodology from
   [`docs/aspects/performance.md`](../aspects/performance.md): idle CPU per frame and
   edit-to-applied latency are the two numbers that must both improve.

## capture.live.provider-unification (P3 · medium) {#capture-live-provider-unification}

**Current state.** The application branches on file-versus-live throughout its frame path. The
state is now explicit in `GraphRunLifecycle` and `CaptureAnalysisLifecycle`, but shell composition
still chooses which owner to service. File sources and live sources publish artifacts and attach
viewer data through different code paths.

**Direction — investigation first, contract second.**

1. Inventory every `App` branch that distinguishes the two worlds (searching for uses of the
   field groups above is the fastest map). Classify each: presentation, readiness, cache/index
   availability, data access, or acquisition control.
2. The first four categories become one provider contract; acquisition control is an *optional
   capability* the provider advertises (file providers do not pretend to support it — the
   `CaptureSourceLifecycle` flags in platform's source metadata already model this shape).
3. Define the contract as a UI-owned port first — unifying `App`'s two worlds is the immediate
   payoff and requires no cross-crate design. Only push the contract down into
   `signal_capture_session` once the UI shape has stabilized and the multi-source viewer items
   (`viewer.multiple-sources`, `viewer.live-snapshots`) confirm what it must carry.
4. Use the existing `CaptureAnalysisLifecycle` boundary to keep the branch inventory and the
   eventual provider adaptation outside `App`'s shell fields.

## performance.regression-harness (P3 · medium) {#performance-regression-harness}

**Current state.** One Criterion-style bench (`benches/compiler_capture.rs`) and three focused
benchmark binaries live in the top-level package alongside ad-hoc `logic-conduit run … --json`
comparisons. The acceptance rule is documented in
[`docs/aspects/performance.md`](../aspects/performance.md).

**Direction.** A comparison *runner*, not more benchmarks — a bin target in
`logic-analyzer-examples`:

1. Input: a workload spec (graph JSON path, capture path via environment/flag since the large
   reference captures are not in the repository — keep them out of ordinary tests) and a
   baseline JSON file.
2. Per run: fixed warmup count, then N measured runs of `logic-conduit run <graph> --json`,
   capturing the report's artifact counts, byte totals, fingerprints, wall time, plus peak RSS
   and CPU time from process accounting.
3. Output: median and spread per metric, exact-identity comparison (fingerprints, word counts,
   bytes — any mismatch is a hard failure, not a statistic), and a stored baseline with metadata
   (git commit, date, host, capture identity). Alternating A/B ordering between two binaries when
   comparing builds.
4. Wire the viewer-latency percentiles in only if cheap; otherwise record that they remain a
   manual step. The harness must make it *hard to accept* a noisy improvement — refuse a
   "retain" verdict when spread overlaps — which is the actual requirement from the TODO item.

## naming.implementation-files (P3 · low) {#naming-implementation-files}

46 files named `implementation.rs`. Mechanical, low risk, high navigation payoff. Per module:
`git mv` the file to a name describing what it holds (the module doc comment's first noun is
usually right — e.g. a `live_capture/implementation.rs` holding the acquisition contract impl
becomes something like `acquisition.rs`), update the `mod` declaration in the owning `mod.rs`,
touch nothing else. No visibility or re-export changes. Batch by crate (one PR per crate is
plenty); expect string architecture tests that `include_str!` a sibling by name to need the
matching one-line update. Skip any file the decomposition items above are about to dissolve —
do those last.
