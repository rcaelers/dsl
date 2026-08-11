# P3 Refactoring Directions

Companion to [P1/P2 Refactoring Directions](refactoring_p1_p2.md); the ground rules there (read
`AGENTS.md`, one item per PR, relocation not redesign, test commands, updating string
architecture tests) apply to every item here and are not repeated. [`TODO.md`](../../TODO.md)
owns priorities and ordering constraints; delete each section when its item completes and the
outcome is documented.

P3 items are planned work, often alongside related changes. The module-ownership rules in
[`responsibility_visibility.md`](../aspects/responsibility_visibility.md#module-ownership) guide
the remaining UI decompositions.

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
