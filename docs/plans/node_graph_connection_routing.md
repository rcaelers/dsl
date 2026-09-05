# Node graph connection routing implementation plan

Owner: `graph.editor.connection-routing` in [TODO.md](../../TODO.md).
Design: [ordered obstacle-avoiding routing proposal](../../writing-block.md).

This is proposed feature work. Each numbered step is one branch; commit and verify it
before starting the next. Dependent branches may be stacked and merge in dependency order.
Do not combine unrelated refactorings. Preserve current wire
appearance until the checked router is activated in step 3.

## 1. Shared path geometry and interaction

- [x] Add the private `widget/graph/routing/` facade and geometry leaves in `node_graph`.
  Introduce line/cubic paths, bounds, adaptive flattening, and queries. Follow the
  owner-facade declaration and visibility rules.
- [x] Adapt existing endpoint Béziers without changing control points. Build a shared
  snapshot for painting, proximity, knife cuts, reroute insertion, and splice detection.
  Preserve preview direction and internal pass-through decoration.
- [x] Test multi-segment paint/interaction consistency, zoom-dependent hit tolerance,
  and existing gestures. Acceptance: external wire interactions no longer reconstruct
  a separate endpoint-only curve, and existing appearance is preserved.

## 2. Checked individual routing

- [x] Add layout-to-geometry adaptation, finite input validation, expanded obstacles,
  endpoint-only escape exemptions, configuration, and classified route outcomes.
- [x] Implement monotonic slab search and the non-monotonic visibility fallback with
  stable ties, appropriate direction/position state, and work limits.
- [x] Add conservative collision checks and validated line-path output. Include offscreen
  nodes and exclude frames. Keep solver activation for the next step.
- [x] Test backward/equal-X paths, blocked escapes, overlap, corner contacts, impossible
  layouts, and work exhaustion. Acceptance: successful paths satisfy endpoint/obstacle
  constraints; failures are explicit and distinguish geometric failure from budget limits.

## 3. Editor activation and compatibility

- [x] Integrate individual results into the shared snapshot. Add visible diagnostic fallback
  treatment and hover explanations, with automatic recovery after geometry edits.
- [x] Use transient endpoint-pair keys within a topology generation. Clear history on
  topology edits, load, undo, and redo. Preserve reroute nodes and branching as documented.
- [x] Preserve node-on-wire splicing through the provisional candidate-obstacle exception.
  Cover visibility, collapse, dynamic sizing, and variadic socket-index changes.
- [x] Add obstacle/escape/result overlays and visual fixtures. Acceptance: detoured wires
  can be cut and edited where drawn; failed wires remain editable; routing alone changes
  neither saved topology, undo history, processing revisions, nor processing outputs.

## 4. Compatible bundles and capacity

- [x] Group eligible node-pair connections deterministically and partition inversions into
  compatible candidates, with destination ordering for shared outputs and stable socket-key
  ties. Bound partition comparisons and retain individual routing until capacity is checked.
- [x] Route rectilinear shared-output fan-out with zero initial separation.
- [x] Search horizontal lane-band envelopes and both connecting fan openings. Allocate fixed
  minimum-spacing lanes and split failed groups deterministically down to individual paths.
- [x] Extend shared envelope search across multiple slabs with interior turns, preserving
  capacity through every connecting opening rather than splitting whenever one band cannot fit.
- [x] Test candidate partitioning for inversions, equal-Y ties, shared outputs, input iteration
  permutations, ineligible geometry, and exhausted grouping comparisons.
- [x] Test routed shared outputs, narrow bands, blocked fan openings, and editor split/retry.
- [x] Test multi-turn shared corridors and their openings. Acceptance: compatible shared interiors preserve order and capacity;
  crossings between separate groups are allowed and not misreported as ordering failures.

## 5. Smooth curves and variable spacing

- [x] Add conservative cubic collision proof with outward-rounded hull subdivision and
  a separate bounded quality budget. Activate eligible individual interior-corner rounding,
  preserve endpoint escapes, and retain checked line paths when quality work cannot finish.
- [x] Test tangency, tight corners, asymmetric clearance, quality-work exhaustion, and
  zoom-scaled interactions on rounded individual paths.
- [x] Add common monotonic X cubic sections with ordered Y control coefficients, matching
  zero Y(X) derivatives at joins, and shared-socket endpoint fan-outs. Validate curves
  conservatively and restore the whole checked bundle when quality cannot be proved.
- [x] Reserve preferred uniform spacing and smoothing clearance before displayed corridor
  commitment, using budget-isolated searches and retaining a checked minimum-spacing route.
- [x] Vary centerline offsets/spacing along a corridor with local capacity. Start from a
  checked spacing lower bound, widen locally with coefficient-order and adjoining-curve
  clearance proof, and retain narrow sections or split groups that cannot fit at minimum.
- [x] Extend smooth joins across individual endpoint escape transitions with optional
  reserved straight runs, preserving mandatory escapes and checking curves against all
  nodes. Cover left/right port orientations, blocked reservation, and zoom-scaled gestures.
- [x] Test whole-curve ordering, tight corners, varying transition lengths, endpoint fan-out,
  asymmetric clearance, and rejected smoothing. Acceptance: smoothing retains collision
  and ordering guarantees; visual fixtures show smooth ordered bundles where feasible.

## 6. Stability, incremental updates, and performance

- [x] Add portable scale fixtures and record the native CPU-only baseline for routing,
  hover and editor tessellation, including hardware, timing distributions and fallback
  counts in the performance record.
- [ ] Capture a completed browser CPU baseline with a reliably bounded runner. Interactive
  execution produced partial progress but no retained full result. Measure GPU
  upload/presentation and real application frame time separately.
- [x] Reuse identical routing snapshots using complete geometry/configuration/zoom keys,
  immutable shared path data, and unchanged failure classifications. Test invalidation
  and compare native cache-hit cost with the retained forced-rebuild baseline.
- [x] Add valid-history hysteresis and dependency invalidation for incident routes, old/new
  obstacle extents, layout/socket changes, configuration, and topology generations.
  Revalidate prior paths and schedule a broader quality pass after dragging stops.
- [x] Test cold and history-aware determinism, unrelated movement, moving obstacles into
  routes, newly opened corridors, pan/zoom, and bounded-work presentation. Acceptance:
  invalid cached paths are never reported safe; unaffected valid routes remain stable.
- [x] Measure native connected-endpoint drag updates, retained paths, evolving fallbacks,
  and release rebuilds. Split CPU frame timing into widget processing and tessellation;
  retain the measured offscreen hit-target z-order optimization and edge-interaction tests.
- [x] Eliminate reference-fixture cold/release work-limit fallbacks using a conservative
  bundle-validation broad phase, with full-scan equivalence, exemption, boundary, and budget
  tests. Require complete checked results at both native scale sizes with unchanged budgets.
- [x] Defer unnecessary connectivity scans for visible socket layout and undecorated socket
  painting. Retain native frame/layout measurements and visibility/indicator regression tests.
- [x] Index socket hit-target order per node to avoid repeated full-map scans. Preserve
  overlapping-target winners, cover layout/topology changes, and retain native timing evidence.
- [ ] Measure 100/500 and 500/2000 node/connection fixtures on native and wasm. Target routing
  p95 below 8 ms on the smaller fixture. Record misses and follow-up work rather than
  weakening constraints. Retain benchmark evidence in the performance design record.

## Verification for every implementation branch

Run `scripts/sort_use_groups.rb` when imports change and format Rust. Then run tests for
every touched crate (the expected owner is `cargo test -p node-graph`), followed by
`cargo test -p logic-analyzer-examples`, then
`cargo clippy --workspace --all-targets --all-features`, and
`cargo check -p logic-analyzer-app-web --target wasm32-unknown-unknown --all-targets --all-features`.
Run applicable architecture/module checks when adding modules or changing dependencies.
Fix newly introduced warnings. No new target-specific dependency, public module, or
persisted document schema is needed for this plan.

Capture visual fixtures at ordinary, low, and high zoom for steps 3–6. Maintain one
regression matrix rather than tests that merely mirror implementation. The documentation-only
planning change requires link and diff checks, not Rust builds.

## Completion

Complete all six steps and verification gates, define visible behavior for every route
outcome, preserve editing gestures on painted paths, and record native/browser measurements.
Document the resulting architecture in present tense under `docs/aspects/`, remove completed
TODO work and this working plan, and retire the proposal once the durable design has an
owner document. Unmet performance targets remain explicit open work.
