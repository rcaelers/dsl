# P3 Refactoring Directions

Companion to [P1/P2 Refactoring Directions](refactoring_p1_p2.md); the ground rules there (read
`AGENTS.md`, one item per PR, relocation not redesign, test commands, updating string
architecture tests) apply to every item here and are not repeated. [`TODO.md`](../../TODO.md)
owns priorities and ordering constraints; delete each section when its item completes and the
outcome is documented.

P3 items are planned work, often alongside related changes. The module-ownership rules in
[`responsibility_visibility.md`](../aspects/responsibility_visibility.md#module-ownership) guide
the remaining UI decompositions.

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
