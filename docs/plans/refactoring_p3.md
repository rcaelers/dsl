# P3 Refactoring Directions

Companion to [P1/P2 Refactoring Directions](refactoring_p1_p2.md); the ground rules there (read
`AGENTS.md`, one item per PR, relocation not redesign, test commands, updating string
architecture tests) apply to every item here and are not repeated. [`TODO.md`](../../TODO.md)
owns priorities and ordering constraints; delete each section when its item completes and the
outcome is documented.

P3 items are planned work, often alongside related changes. Ordering that matters:
[ui.graph-service.port-shape](#ui-graph-service-port-shape) before
[ui.app.decomposition](#ui-app-decomposition); the
[ui.boundaries.module-ownership](#ui-boundaries-module-ownership) rules before or alongside both
UI decompositions.

## ui.graph-service.port-shape (P3 · medium) {#ui-graph-service-port-shape}

**Current state.** `GraphService` is a `pub(crate)` trait
(`crates/logic_analyzer_ui/src/graph_service/contract.rs:94`) with two implementations:
`UiGraphService` (`graph_service/graph_compiler.rs:287`) and `FakeGraphService` in
`graph_service/graph_service_tests.rs`. The contract is typed in compiler, runtime, plan,
orchestration, and capability types, so the trait hides nothing — the UI manifest carries all six
graph-crate dependencies regardless.

**Recorded recommendation: remove the trait.** The UI owns graph execution; document that.

**Steps.**

1. Check what `FakeGraphService` actually fakes in the UI component tests. The testing strategy
   already says UI tests use local implementations — the question is at what level. The cleaner
   seam is *below* the service: construct the real `UiGraphService` over the in-memory
   `ArtifactRepository` (`platform_artifacts` provides one) and controlled executors, which the
   graph-runtime tests already do. If a handful of tests genuinely need to stub whole-service
   behavior, keep a minimal `#[cfg(test)]` trait for those tests only — not a production
   abstraction.
2. Change `App.graph_service: Box<dyn GraphService>` (`app.rs:547`) to the concrete
   `UiGraphService`; inline or delete `contract.rs`; keep `CaptureFeatureDiscovery` (the
   supertrait) only if something else implements it.
3. While touching the contract: do not widen it. The four `Result<_, String>` methods on it are
   [errors.typed-boundaries](#errors-typed-boundaries) work; leave them unless that item is being
   done in the same series.
4. Update `docs/architecture/crate_responsibility.md` ("Application coordination" section) to say
   the UI owns a concrete graph service rather than a port.

**Acceptance.** No `dyn GraphService` in production code; UI tests pass against the real service
with injected repositories/executors.

## ui.app.decomposition (P3 · high) {#ui-app-decomposition}

**Current state.** `pub struct App` (`crates/logic_analyzer_ui/src/app.rs:543`) has 47 fields in
a 4,390-line file. The fields, grouped by the owner they should move to:

- **Graph-run lifecycle** — `graph_service`, `run`, `run_message`, `running_graph_semantics`,
  `cached_preview_graph`, `last_live_sync`, `sampling_overlay_candidates`,
  `derived_cache_clear_task`.
- **Capture-analysis lifecycle** — `capture` (the `CaptureCoordinator`), `capture_availability`,
  `capture_graph`, `capture_analysis`, `capture_analysis_error`, `capture_epoch_observed_graph`,
  `capture_epoch_request_in_flight`, `last_capture_epoch_sync`, `trigger_configuration`,
  `trigger_configuration_error`, `capture_storage`.
- **Presentation catalogs** — `presented_derived_lanes`, `output_presentation_catalog`,
  `table_presentation_catalog`, `presentation_graph_nodes`, `decoder_panels`, `plugin_panels`,
  `viewer_lane_order`, `selected_sampling_overlays`.
- **Timeline-marker bindings** — `timeline_marker_owners`, `timeline_marker_error`,
  `timeline_marker_reference_error`.
- **Shell (stays on `App`)** — `node_graph`, `logic_analyzer`, `panel_layout`, `input_bindings`,
  `host_service`, `host_ui_capabilities`, `toasts`, `platform`, `about`, `output_downloads`,
  `preferences`, `node_catalogs`, `demo_graphs`, `error_badges`, `memory_panel`,
  `_worker_operation_executor`.

**Method.** Extract one group per PR, smallest first: timeline markers (3 fields), then
presentation catalogs, then graph-run, then capture-analysis (largest, and partly gated on the
[coordinator split](#ui-capture-coordinator-decomposition)). For each group:

1. Create an owned struct in its own module under `logic_analyzer_ui` (directory-backed, facade
   rules apply). Fields private; state transitions become methods; the struct's doc comment
   states its invariants (e.g. "`capture_analysis_error` is `Some` only when `capture_analysis`
   is `None`").
2. Move the private `App` methods that touch only this group. Let the borrow checker drive the
   remainder: a method touching two groups becomes a method on one group taking the other (or a
   narrow view of it) as an argument — that argument list *is* the documented coupling between
   owners. Do not pass `&mut App` back in.
3. `App` keeps composition and per-frame dispatch: its `update` calls each owner once with what
   that owner declares it needs.

**Do not** redesign behavior, rename user-visible anything, or change persistence formats
(`SavedPanelLayout`, `SavedTimelineCursors`, … at the top of `app.rs` are persisted contracts).

## ui.capture.coordinator-decomposition (P3 · high) {#ui-capture-coordinator-decomposition}

**Current state.** `crates/logic_analyzer_ui/src/live_capture/coordinator.rs` is 2,867 lines.
Its internal types already name the seams: `CaptureCommand`, `PersistedConfigurationEpoch`,
`WorkerPreparedConfigurationEpoch`, `CaptureWorkerSession`/`CaptureWorkerPorts`,
`ActiveCapture`, `CompletedCapture`, `PinnedCaptureIndex`, `RecordingEventPublisher`,
`write_application_metadata`, with `CaptureCoordinator` at line 293, its main `impl` at 312, and
the `CaptureCoordinatorContract` impl at 838.

**Split along the lines the types already draw**, as sibling leaf modules inside `live_capture`:

- *Acquisition state machine*: commands, configuration epochs, the worker session, active/
  completed capture transitions. Owns the invariant that only one capture is active and that
  epoch acknowledgements are ordered.
- *Storage publication*: `PinnedCaptureIndex`, `write_application_metadata`, session-repository
  interaction — everything that turns a completed capture into published artifacts.
- *Status projection*: the snapshot/status types the UI reads. Projection reads the state
  machine; the state machine never formats for display.
- `CaptureCoordinator` remains as the thin composition of the three, keeping
  `CaptureCoordinatorContract` stable so `App` (and later the extracted capture-analysis owner)
  does not change in the same PR.

Also: `TestWorkExecutor` and `test_work_executor()` sit at the top of the production file
(lines 46–68) — move them into a `…tests` module or the test-support crate first; that is a
trivial standalone PR.

## ui.boundaries.module-ownership (P3 · medium) {#ui-boundaries-module-ownership}

**What it is.** A documentation-rule change, then applying it. `docs/aspects/
responsibility_visibility.md` stops at the crate wall; the two files above show why that is not
enough.

**Steps.** Add a "Module ownership" section to `responsibility_visibility.md`: any module that
exceeds roughly 1,000 lines or owns cross-cutting mutable state must answer the four owner
questions (data/invariants, supported facade, permitted dependencies, exclusions) in its module
doc comment — the same four questions `crate_responsibility.md` already poses. State that a
module which cannot answer them concisely is a decomposition candidate. Then write those doc
comments for the modules the two decomposition items create, and for the three or four largest
existing modules that survive. Keep the threshold advisory, not a hard lint.

## errors.typed-boundaries (P3 · medium) {#errors-typed-boundaries}

The remaining string-error surfaces are concentrated in platform, UI, graph nodes, processing,
`platform_runtime`, and `signal_runtime`. `platform_runtime` already owns `WorkerMessageError`;
`signal_runtime` owns `PortError`, `ConnectionError`, and `WorkError`. Extend those owner-specific
surfaces rather than replacing them with an umbrella error.

**How to type an error here** (`thiserror` is already a workspace dependency):

- One enum per *facade*, not per crate and not per function. Variants describe what failed in
  the owner's vocabulary; a wrapped lower-level error rides in a variant field.
- Convert at ownership boundaries: a crate maps a dependency's error into its own variant
  (`#[from]` only when the dependency type is itself part of the crate's contract). Never
  `format!` a message and pass it up — formatting happens once, at the presentation boundary.
- `Result<_, String>` in *tests* is fine; do not churn test code.

**Order (work outward from the lowest owner, per the TODO item):**

1. `platform_runtime`: executor, task, worker-message, kernel-registration, and queue paths that
   still return `String`.
2. `signal_runtime`: manager and pipeline-supervision paths that still return `String`.
   Downstream crates then hold typed sources to wrap.
3. Host-override contracts — `SigrokDecoderRuntime::{discover,create}` and
   `SigrokCatalogScanner` — in their `logic_analyzer_protocol_decoders` owner, so the error types are
   defined once in their final home.
4. `graph_runtime` source preparation: give `SourcePreparationUpdate::Failed` a typed cause and
   find the UI code that currently distinguishes failures by message text (search `app.rs` and
   the run-message path for string matching on error content) — each such site becomes a match
   on a variant.
4. Platform and UI last: most of their 170 occurrences will collapse into carrying the
   now-typed lower errors; only genuinely UI-owned failures need new variants.

Expect this to span many small PRs; each facade conversion is independently landable.

## session.domain-relocation (P3 · medium) {#session-domain-relocation}

The recorded signal-tier vocabulary makes this an ownership purge. The trigger cluster (trigger
program, trigger schema, `SimpleTriggerCondition`) and the public `logic_analyzer` facade are
product-domain APIs inside a generic acquisition-session crate. The trigger types are consumed by
`logic_analyzer_viewer`, `logic_analyzer_graph_compiler`, `trigger_editor`, and the UI, so those
consumers currently reach through the session owner for unrelated data contracts.

**Direction.**

1. Extract the serializable trigger program, schema, predicates, and control vocabulary into a
   small `logic-analyzer-trigger` domain crate. It depends only on neutral value contracts and is
   usable by the viewer, editor, compiler, concrete acquisition, and UI.
2. Move the remaining driver/source contracts from `signal_capture_session::logic_analyzer` to
   their logic-analyzer acquisition owners. Coordinate contracts shared by multiple devices with
   the processing-domain split rather than making the generic session crate depend upward.
3. Retain only acquisition lifecycle, integrity, bounded delivery, recording, and storage
   coordination in `signal_capture_session`. Remove the `logic_analyzer` facade without a
   compatibility re-export through the generic crate.
4. Fix `session.facade-glob` opportunistically while curating the reduced facade.

Acceptance: `signal_capture_session` exposes no logic-analyzer trigger, device, source, or driver
vocabulary; it has no `logic_analyzer_*` dependency; and the viewer and trigger editor depend on
the trigger owner rather than `signal-capture-session`.

## derived.payload.builtin-registration (P3 · medium) {#derived-payload-builtin-registration}

The recorded signal-tier vocabulary makes this both an ownership purge and a registration-path
correction. `logic_analyzer_graph_registry` provides an open `PayloadRegistration` inventory, and
the built-in registrations already exist in
`crates/logic_analyzer_graph_nodes/src/payloads/{digital,word,trigger}.rs`. In parallel,
`signal_derived` owns a closed built-in set in
`src/derived_data_collector/{digital,word,trigger}.rs` and exports the corresponding adapters.

**Direction.**

1. Classify the existing built-ins by responsibility. Generic retained-value, query, index, and
   storage contracts remain in `signal_derived`; `TriggerLaneSnapshot`, `ProtocolPacket`, and
   other product trigger/protocol semantics move to their logic-analyzer domain owners. A type
   does not remain generic merely because several built-in nodes consume it.
2. Remove concrete built-in branches from generic collection and query code. Extend the existing
   type-erased adapter contract where necessary so externally owned payloads can supply ingestion,
   snapshots, and persistent storage without adding a reverse dependency.
3. Route every built-in through its registered adapter, following the same path as the example
   plugin's camera payload. Registrations in the concrete feature owner become the only way those
   payloads enter a registry snapshot.
4. Remove trigger/protocol adapter exports and compatibility aliases from `signal_derived` once
   consumers import their actual owner.

Acceptance: the example-plugin payload and a built-in payload traverse identical code paths from
registration to collection; `signal_derived` exposes no logic-analyzer trigger or decoded-protocol
type; and no generic crate branches on a built-in payload identity.

## graph.execution.debounced-live-sync (P3 · medium) {#graph-execution-debounced-live-sync}

**Current state.** `app.rs:2772` — `const SYNC_INTERVAL_S: f64 = 0.5`: every 0.5 s the UI thread
computes `self.node_graph.graph().semantic_snapshot()` and compares it against
`cached_preview_graph` (`app.rs:2784`), then refreshes capture availability, trigger
configuration, and sampling-overlay candidates. A parallel 0.5 s epoch poll exists at
`app.rs:2421` (`EPOCH_SYNC_INTERVAL_S`). Cost is paid when idle; latency is paid when editing.

**Direction.**

1. Add a monotonically increasing *semantic revision* to the graph document, bumped only by
   processing-relevant edits (node/connection/state changes — not node positions or panel
   state). Home: the document model — coordinate with `graph.document-model-extraction` (P2) so
   the revision lands in the extracted crate if that has happened first; otherwise in
   `node_graph::model` with the same semantics.
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

**Current state.** The application branches on file-versus-live throughout its frame path: `App`
holds a parallel pair of worlds (`run`/`run_message` versus `capture`/`capture_graph`/
`capture_analysis`), and code like `if self.run.is_none() { if self.capture.is_active() … }`
(`app.rs:2778`) chooses which world to service. File sources and live sources publish artifacts
and attach viewer data through different code paths.

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
4. This item pairs naturally with the capture-analysis extraction in
   [ui.app.decomposition](#ui-app-decomposition): extracting that owner first makes the branch
   inventory in step 1 nearly mechanical.

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

## docs.drift-correction (P3 · medium) {#docs-drift-correction}

Two classes, treated differently:

1. **Plainly wrong now** — fix immediately: `AGENTS.md` line 27 documents a `signal_processing`
   crate that no longer exists; replace that bullet with the actual `signal_runtime` /
   `signal_capture` / `signal_derived` / `signal_capture_session` split (one line each, matching
   `crate_responsibility.md`).
2. **Aspirational-but-normative** — the statements that apps are the composition roots and host
   factories are injected (`AGENTS.md`, `crate_responsibility.md`). These are the *target* of the
   P1/P2 composition items; leave the documents normative and land the code. Only if the P1 items
   stall long-term should the docs gain an explicit "not yet true, see TODO" marker — a normative
   doc that silently disagrees with the code is how this drift started.

Sweep for further drift while there: any doc naming obsolete composition facades, global
`install_…` APIs, or the pre-split processing layout will need updating as those items land — each
such PR updates the docs it invalidates, per the ground rules.

## naming.implementation-files (P3 · low) {#naming-implementation-files}

46 files named `implementation.rs`. Mechanical, low risk, high navigation payoff. Per module:
`git mv` the file to a name describing what it holds (the module doc comment's first noun is
usually right — e.g. a `live_capture/implementation.rs` holding the acquisition contract impl
becomes something like `acquisition.rs`), update the `mod` declaration in the owning `mod.rs`,
touch nothing else. No visibility or re-export changes. Batch by crate (one PR per crate is
plenty); expect string architecture tests that `include_str!` a sibling by name to need the
matching one-line update. Skip any file the decomposition items above are about to dissolve —
do those last.
