# Node graph connection routing implementation plan

Owner: `graph.editor.connection-routing` in [TODO.md](../../TODO.md).
Design: [ordered obstacle-avoiding routing proposal](../../writing-block.md).

This is proposed feature work. Each numbered step is one branch; merge and verify it
before starting the next. Do not combine unrelated refactorings. Preserve current wire
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

- [ ] Add layout-to-geometry adaptation, finite input validation, expanded obstacles,
  endpoint-only escape exemptions, configuration, and classified route outcomes.
- [ ] Implement monotonic slab search and the non-monotonic visibility fallback with
  stable ties, appropriate direction/position state, and work limits.
- [ ] Add conservative collision checks and validated line-path output. Include offscreen
  nodes and exclude frames. Keep solver activation for the next step.
- [ ] Test backward/equal-X paths, blocked escapes, overlap, corner contacts, impossible
  layouts, and work exhaustion. Acceptance: successful paths satisfy endpoint/obstacle
  constraints; failures are explicit and distinguish geometric failure from budget limits.

## 3. Editor activation and compatibility

- [ ] Integrate individual results into the shared snapshot. Add visible diagnostic fallback
  treatment and hover explanations, with automatic recovery after geometry edits.
- [ ] Use transient endpoint-pair keys within a topology generation. Clear history on
  topology edits, load, undo, and redo. Preserve reroute nodes and branching as documented.
- [ ] Preserve node-on-wire splicing through the provisional candidate-obstacle exception.
  Cover visibility, collapse, dynamic sizing, and variadic socket-index changes.
- [ ] Add obstacle/escape/result overlays and visual fixtures. Acceptance: detoured wires
  can be cut and edited where drawn; failed wires remain editable; routing alone changes
  neither saved topology, undo history, processing revisions, nor processing outputs.

## 4. Compatible bundles and capacity

- [ ] Group eligible node-pair connections deterministically and partition inversions into
  compatible sub-bundles. Handle shared-output fan-out with zero initial separation.
- [ ] Search with bundle envelopes, checking slabs and connecting openings. Allocate fixed
  minimum-spacing lanes and split failed groups deterministically down to individual paths.
- [ ] Test inversions, equal-Y ties, shared outputs, narrow openings, and input iteration
  permutations. Acceptance: compatible shared interiors preserve order and capacity;
  crossings between separate groups are allowed and not misreported as ordering failures.

## 5. Smooth curves and variable spacing

- [ ] Add centerline/offset geometry with common monotonic X parameterization and ordered
  Y control coefficients. Reserve preferred spacing before corridor commitment; reduce
  toward minimum spacing or split when needed.
- [ ] Add horizontal port/lane tangents and derivative-matched smooth joins. Prove collision
  safety with hull bounds/subdivision. Bound handle retries and retain checked line routes
  when smoothing cannot be proved safe.
- [ ] Test whole-curve ordering, tight corners, varying transition lengths, endpoint fan-out,
  asymmetric clearance, and rejected smoothing. Acceptance: smoothing retains collision
  and ordering guarantees; visual fixtures show smooth ordered bundles where feasible.

## 6. Stability, incremental updates, and performance

- [ ] Establish cold native/browser benchmarks before optimization, including interaction
  cost and complete frame time. Record hardware, fixtures, and timing distributions.
- [ ] Add valid-history hysteresis and dependency invalidation for incident routes, old/new
  obstacle extents, layout/socket changes, configuration, and topology generations.
  Revalidate prior paths and schedule a broader quality pass after dragging stops.
- [ ] Test cold and history-aware determinism, unrelated movement, moving obstacles into
  routes, newly opened corridors, pan/zoom, and bounded-work presentation. Acceptance:
  invalid cached paths are never reported safe; unaffected valid routes remain stable.
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
