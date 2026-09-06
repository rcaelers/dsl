# Performance Design and Measurement Record

## Purpose

This document owns the performance contract that spans capture indexing, derived storage, graph
execution, host I/O, and foreground responsiveness. It records the design commitments that follow
from measurement, the retained baseline, the experiments performed, and the approaches that were
measured and rejected.

Two tenses are used deliberately. The retained design and its rules are stated in present tense and
describe implemented behavior. The investigation record is stated in past tense because it reports
experiments, including reverted ones. A rejected approach is documented here so it is not attempted
again without new evidence; the absence of a record is not evidence that an idea is untried.

Actionable work belongs in [`TODO.md`](../../TODO.md). This document holds the evidence those items
are prioritized from.

## Node-graph routing scale baseline

The portable `routing_performance_tests` fixture measures four CPU-side costs separately:
`rebuild_routes` on a prepared layout, exact-input cache reuse, an average of 32 point-to-wire hover queries, and
`Context::run_ui` plus editor rendering and egui tessellation. The CPU frame includes the
editor's layout/routing passes, but excludes texture uploads, GPU submission, presentation,
and the surrounding application. It is not an end-to-end display-frame measurement.

`paired-grid-v1` has 100 nodes/500 connections or 500 nodes/2000 connections. Each neutral
node has ten inputs and ten outputs. Disjoint source/target pairs occupy five columns,
900 units apart horizontally and 700 vertically; each target is 450 units right of its
source. The smaller fixture connects all ten matching ports per pair, the larger eight.
A fixture assertion keeps every body and its 60-unit escape envelope disjoint. Zoom is
0.35 and the logical egui viewport is 1440 × 900, so routing includes offscreen geometry.

The isolated routing timer forces a rebuild without history; CPU frames follow production
snapshot reuse. Layout preparation, including its initial route build, is outside the
isolated routing timer. The first measured sample is recorded
separately; subsequent samples use warmed allocator/font/egui state, not a cold process.
Release runs collect twenty subsequent samples and report nearest-rank p50/p95, maximum,
and sorted raw samples. Debug tests run a cold/repeated two-sample correctness smoke check.
Every sample asserts complete finite path presentation, stable routing outcome counts, and
unchanged topology sizes; there are no hardware-dependent timing assertions. Fallback
counts and reasons accompany timings so work exhaustion cannot masquerade as throughput.
The exact-input reuse measurement additionally times a cache hit on the same prepared
layout, including complete key construction/comparison and copying the path/failure maps
with shared immutable geometry. It asserts that the stationary fixture actually hits
the cache. The original native baseline predates snapshot reuse and retains its original
field set; cache measurements report `cached_routing` separately from forced `routing`.
The frame breakdown also reports a separate `build_layout` sample (with production
history), `Context::run_ui` duration, and tessellation duration. The latter two are timed
within the CPU frame; the separate layout sample is not an additional frame component.

Reproduce the native measurement with:

```sh
cargo test -p node-graph --release routing_scale_native -- --nocapture
```

The browser uses the same stationary and connected-drag fixture bodies. Build the release
test binary, then pass the `.wasm` executable path printed by Cargo to the bounded runner:

```sh
cargo test -p node-graph --release --target wasm32-unknown-unknown --lib --no-run
CHROME_BIN="/path/to/chrome" WASM_BINDGEN_TEST_RUNNER=wasm-bindgen-test-runner \
  node scripts/measure_node_graph_browser.mjs /path/to/node_graph-test.wasm
```

The host script requires Node 22 or newer and Chrome on macOS/Linux. The wasm-bindgen runner
must match the lockfile version (0.2.127 for this baseline). It serves tests on an ephemeral
loopback port and starts a separate headless Chrome process with a fresh temporary profile;
it never attaches to an existing browser. DevTools captures console reports and the final
test summary without virtual-time overrides. `ROUTING_PROGRESS` messages go to stderr,
outside measured intervals. Successful stdout JSON requires both browser tests to pass and
both fixtures to retain twenty release samples, including isolated drag routing and the
pointer-driven UI/tessellation/frame timers, drag outcomes, and start/release/settled phases.
Twenty repeated release/idle cycles must also have complete route outcomes and chronological
frame timings that agree exactly with their sorted distributions.
Debug, partial, or failed executions are not accepted as baselines.

`ROUTING_BROWSER_TIMEOUT_SECONDS` bounds the complete serve/browser execution, defaulting
to 180 seconds, independently of the browser event loop; compilation is separate. Timeout,
interruption, early process exit, or connection loss fails the run. Cleanup terminates only
the process groups it starts and removes their temporary profile. Runner regression tests
include a stalled process with a descendant and assert that neither survives the timeout:
`node --test scripts/measure_node_graph_browser_test.mjs`. This harness has no GPU-backed
canvas and does not measure texture upload, presentation, or surrounding application work.

The native reference was measured on 2026-09-05 using the routing implementation at
`8da42b1d`, an Apple M1 Ultra (20 logical CPUs, 64 GiB), macOS 26.6.2 (25G83), Rust
1.100.0-nightly (0ed41eb41, 2026-09-04), and the release profile with debug information.
Raw native measurements are retained in
[`node_graph_routing_native_baseline.json`](../../benchmarks/performance/node_graph_routing_native_baseline.json).

| Native fixture | Routing p50 / p95 | Hover p50 / p95 | CPU frame p50 / p95 | Fallbacks |
| --- | --- | --- | --- | --- |
| 100 nodes / 500 connections | 1.69 / 1.76 ms | 0.045 / 0.053 ms | 24.61 / 25.50 ms | 0 |
| 500 nodes / 2000 connections | 9.03 / 9.41 ms | 0.198 / 0.242 ms | 524.87 / 535.00 ms | 1360 (`WorkLimit`) |

The smaller native fixture meets the 8 ms routing-p95 target without fallbacks. The larger
fixture does not complete checked routing for every connection within the work budget;
its timing is not full-route throughput. CPU frame processing is substantially more
expensive than isolated routing. Those costs require separate profiling during step 6;
increasing the work budget alone is not an established remedy.

The exact-input cache comparison uses the same hardware, profile, and fixture. Its raw
samples are retained in
[`node_graph_routing_exact_cache.json`](../../benchmarks/performance/node_graph_routing_exact_cache.json).
Workspace test validation ran concurrently with this capture, so the timings are
observational evidence, not an isolated regression threshold or a controlled speedup ratio.

| Native fixture | Cache hit p50 / p95 | Forced rebuild p50 / p95 | CPU frame p50 / p95 | Fallbacks |
| --- | --- | --- | --- | --- |
| 100 nodes / 500 connections | 0.079 / 0.216 ms | 1.74 / 1.90 ms | 20.79 / 21.76 ms | 0 |
| 500 nodes / 2000 connections | 0.228 / 0.410 ms | 8.99 / 9.30 ms | 499.33 / 515.18 ms | 1360 (`WorkLimit`) |

Identical-input reuse avoids solver work but retains all failure classifications. It does
not resolve the larger fixture's checked-route shortfall or CPU frame cost, and these
stationary measurements do not establish moving-node or browser responsiveness.

Interactive Chrome execution reached both corrected fixtures, but automation stalled
before a completed test result and full JSON report could be retrieved. Partial progress
logs are not retained as a browser baseline or a browser correctness pass. Repeating the
measurement with the bounded runner provides the complete
[browser CPU reference](#bounded-browser-and-native-cpu-reference) below.

### Proposed future measurements

Real application/GPU frame timing and broader drag scenarios remain required by the
connection-routing plan. The CPU-only
scale and connected-endpoint fixtures do not replace those acceptance gates.

### Connected-endpoint drag measurement

`routing_drag_native` and `routing_drag_browser` use `paired-grid-connected-drag-v1`:
the same two graph sizes and 0.35 zoom, with the first connected source alternating
between its original Y and Y + 20. The other nodes remain stationary. Each sample times
layout preparation with production history, an independent history-aware route update on
that prepared layout, and a forced cold rebuild. Preparation does not warm the independent
route history. Every sample changes geometry, checks finite complete presentation,
verifies that incident routes are rebuilt, and counts shared checked paths and failures.
The final release rebuild is checked against the same geometry's cold result. These are
route/layout measurements, not input-dispatch or complete drag-frame timings.

The first update is separate from twenty subsequent release-profile samples; the latter
report nearest-rank p50/p95/p99 and raw samples. With only twenty samples, p99 is the maximum,
not a well-estimated long-tail percentile. Debug tests use two updates without timing gates.
Run `cargo test -p node-graph --release routing_drag_native -- --nocapture` to reproduce.

The native capture on 2026-09-05 uses the reference M1 Ultra host/profile above and routing
revision `5a0bbe24`. Raw timings and per-update outcomes are in
[`node_graph_routing_drag_native.json`](../../benchmarks/performance/node_graph_routing_drag_native.json).

| Native fixture | Drag route p50 / p95 / p99 | Cold route p95 | Layout p95 | Release rebuild |
| --- | --- | --- | --- | --- |
| 100 nodes / 500 connections | 0.373 / 0.435 / 0.450 ms | 1.805 ms | 1.362 ms | 1.731 ms |
| 500 nodes / 2000 connections | 1.191 / 2.415 / 8.776 ms | 9.048 ms | 10.336 ms | 8.983 ms |

The smaller fixture retains 490 unrelated checked paths on every move, with no failures.
The larger starts with 1360 cold `WorkLimit` fallbacks; its first three updates reduce those
to 728, 96, and zero as retained proofs free search work for other pairs. Later moves retain
1992 paths. Cold routing and the release quality rebuild still produce 1360 fallbacks.
Warm recovery therefore does not establish complete cold-route throughput, and the release
shortfall remains required follow-up work.

### Offscreen hit-target z-order cost

The CPU frame breakdown places most of the larger stationary fixture's cost in widget
processing, not tessellation: before the change, UI p95 is 521.83 ms and tessellation p95
is 1.32 ms. egui's `move_to_top` implementation removes and reinserts a widget in its layer
and updates the shifted widget indexes. Raising every offscreen target repeats that work
for targets that cannot cover a visible inline control.

The editor retains initial response registrations but skips the second, z-order-only
registration when that target is outside the drawing clip. Each socket target is checked
independently of its node body, preserving protruding socket hit areas at viewport edges.
Complete layout, obstacle geometry, routing, and document state are unchanged.

Sequential native runs on the same reference host, without concurrent cargo validation,
give the following observations. These are not randomized paired trials or end-to-end GPU
measurements. Full before/after samples are in
[`node_graph_hit_target_culling_native.json`](../../benchmarks/performance/node_graph_hit_target_culling_native.json).

| Native fixture | CPU frame p95 before / after | Routing fallback count before / after |
| --- | --- | --- |
| 100 nodes / 500 connections | 21.90 / 9.85 ms | 0 / 0 |
| 500 nodes / 2000 connections | 523.10 / 63.36 ms | 1360 / 1360 |

The larger frame remains too expensive for smooth interaction. Further profiling and
application/GPU measurements remain necessary; this optimization does not change
routing work budgets or reinterpret failures as checked paths.

### Complete cold routing with a bundle-validation broad phase

Exact bundle validation selects expanded obstacles intersecting a closed envelope around
all candidate lanes, fan-outs, and endpoint escapes. It scans every input obstacle once,
retains original exemption indexes, and rejects queries outside that envelope. Every
potential collider still receives the same exact closed-rectangle segment test. The
selection scan and subsequent checks spend the existing work allowance; no budget is raised.

On the reference M1 Ultra host/profile, the 2026-09-05 capture with this change completes
both scale fixtures with zero fallbacks, including cold routes, each connected-endpoint
move, and the final release rebuild. The tests require this result without hardware timing
assertions. Raw distributions and per-move outcomes are in
[`node_graph_bundle_broad_phase_native.json`](../../benchmarks/performance/node_graph_bundle_broad_phase_native.json).

| Native fixture | Cold route p50 / p95 | Drag route p95 | Release rebuild | CPU frame p95 | Fallbacks |
| --- | --- | --- | --- | --- | --- |
| 100 nodes / 500 connections | 1.182 / 1.266 ms | 0.421 ms | 1.275 ms | 9.531 ms | 0 |
| 500 nodes / 2000 connections | 12.487 / 12.818 ms | 1.386 ms | 12.951 ms | 62.514 ms | 0 |

The larger cold pass takes longer than the earlier roughly 9 ms work-limited measurement,
but completes all 2000 checked routes instead of abandoning 1360 of them. This is a
completeness improvement, not a like-for-like latency regression or a claim of faster
full-route throughput against that incomplete baseline. The smaller fixture remains below
the 8 ms routing-p95 target. Large-frame cost, broader layouts, and
application/GPU measurements remain open acceptance work. The full-scan collision oracle,
boundary contacts, nonfinite geometry, original endpoint exemptions, envelope containment,
and exhausted-work behavior have regression coverage.

### Deferred socket connectivity queries

Already-visible socket layout does not need a connection-list scan: connectivity affects
visibility only when the socket would otherwise be hidden. Indicator painting likewise
needs connectivity for placement only when that socket has at least one decoration.
The editor defers both queries until needed, without retaining a connectivity cache or
changing the visibility truth table, indicator owner order, or connected placement offsets.

Sequential native before/after runs on the reference M1 Ultra host/profile on 2026-09-05
give the following observations. There is no concurrent cargo validation, but these are
not randomized paired trials or end-to-end GPU/application measurements. Raw samples are in
[`node_graph_lazy_socket_queries_native.json`](../../benchmarks/performance/node_graph_lazy_socket_queries_native.json).

| Native fixture | Layout p95 before / after | CPU frame p95 before / after | Fallbacks before / after |
| --- | --- | --- | --- |
| 100 nodes / 500 connections | 0.742 / 0.438 ms | 9.302 / 8.756 ms | 0 / 0 |
| 500 nodes / 2000 connections | 7.802 / 1.996 ms | 61.752 / 45.491 ms | 0 / 0 |

Routing outcome counts, including cubic counts, are unchanged. The fixture has visible
sockets and no indicators, so it exercises the avoided scans; it does not establish the
same improvement for heavily hidden or decorated graphs. Visibility truth-table and
multi-owner indicator placement tests cover hidden/connected sockets, connect/disconnect
updates, and low/normal/high zoom. Large-frame cost and application/GPU measurements
remain open acceptance work.

### Per-node socket hit-order index

Each layout derives a per-node socket index from the flat hit-target map, preserving the
relative iteration order used for overlapping targets. Vectors reserve capacity from node
socket counts. The z-order pass reads only each node's own socket identities and resolves
their rectangles in the same snapshot. Thus 500 nodes with 10000 socket targets require
10000 socket visits rather than 5000000 full-map visits during node raising. Initial
response allocation, clipping, painted node order, routing, and saved graphs are unchanged.

Sequential native measurements on the reference host/profile on 2026-09-05 show the tradeoff:
building the index adds layout work while eliminating repeated scans in the frame. Full
samples are in
[`node_graph_socket_hit_index_native.json`](../../benchmarks/performance/node_graph_socket_hit_index_native.json).
Source stays fixed through each compile/run and there is no concurrent cargo validation;
these are observational before/after runs, not randomized paired trials or GPU measurements.

| Native fixture | Layout p95 before / after | CPU frame p95 before / after | Fallbacks before / after |
| --- | --- | --- | --- |
| 100 nodes / 500 connections | 0.419 / 0.476 ms | 7.891 / 7.863 ms | 0 / 0 |
| 500 nodes / 2000 connections | 1.935 / 2.087 ms | 44.956 / 34.749 ms | 0 / 0 |

The smaller frame result is essentially unchanged; the larger benefits from eliminating
the repeated visits. Routing outcome and cubic counts are unchanged. Tests compare the
index with the flat-map projection across hiding, collapse, socket growth, connection and
node changes, and verify the same winner for deliberately overlapping socket targets.
The larger CPU frame remains above a 60 Hz budget, and application/GPU verification
remains outstanding.

### Bounded browser and native CPU reference

The release fixture at `04c45237` completes both browser tests in isolated headless Chrome
152.0.7977.83 (V8 15.2.124.21) on the reference M1 Ultra host. The retained 2026-09-05 run uses
a 45-second outer deadline and matching wasm-bindgen-test-runner 0.2.127. Separate native
stationary and drag runs follow on the same Rust source without concurrent cargo validation.
The full browser identity, test summary, wasm hash, host metadata, first samples, and subsequent
sample arrays are retained in
[`node_graph_routing_browser_baseline.json`](../../benchmarks/performance/node_graph_routing_browser_baseline.json).

| Target / fixture | Forced routing p95 | Cache-hit p95 | CPU frame p95 | Fallbacks |
| --- | --- | --- | --- | --- |
| Native, 100 nodes / 500 connections | 1.266 ms | 0.153 ms | 8.315 ms | 0 |
| Chrome, 100 nodes / 500 connections | 2.515 ms | 0.120 ms | 12.885 ms | 0 |
| Native, 500 nodes / 2000 connections | 12.620 ms | 0.666 ms | 39.217 ms | 0 |
| Chrome, 500 nodes / 2000 connections | 17.460 ms | 0.525 ms | 58.010 ms | 0 |

| Target / connected-endpoint drag | Update p95 | Cold p95 | Release rebuild | Retained paths |
| --- | --- | --- | --- | --- |
| Native, 100 nodes / 500 connections | 0.432 ms | 1.207 ms | 1.251 ms | 490 |
| Chrome, 100 nodes / 500 connections | 0.530 ms | 2.225 ms | 2.145 ms | 490 |
| Native, 500 nodes / 2000 connections | 1.394 ms | 12.537 ms | 12.634 ms | 1992 |
| Chrome, 500 nodes / 2000 connections | 1.565 ms | 17.515 ms | 16.950 ms | 1992 |

Every recorded warm/cold drag outcome has zero fallbacks; release checks match the cold
geometry and failure map. The smaller fixture meets the 8 ms routing-p95 target on both
targets. The larger CPU frame remains above a 60 Hz budget on both. These single-host
observations are not randomized paired comparisons, a cross-browser guarantee, or evidence
of an end-to-end application frame rate. These connected-drag timers isolate routing/layout;
the pointer-driven measurement below covers the full CPU widget frame. GPU and real
application measurements remain open.

### Pointer-driven drag CPU frames

The drag report includes a separate `paired-grid-pointer-drag-frames-v1` fixture under
`frames`. It uses the same neutral nodes, connection counts, zoom 0.35, and 1440 × 900 egui
viewport. Pan is [150, 100] screen units to keep the source header grip away from auto-pan
edges. This changes visible geometry relative to the stationary fixture, so their frame
times are not like-for-like optimization comparisons.

After initial paint, a primary press and a ten-screen-unit move enter node dragging through
normal input dispatch. Measured pointer movements then alternate the connected source
between its starting position and twenty canvas units below it. A separate widget and
routing history prevent the isolated route-update benchmark from priming changed geometry
before a timed frame. Each timer covers `Context::run_ui`, both widget layout passes,
input handling, route updates, paint generation, and tessellation. UI and tessellation
timings are also retained separately. No texture upload, GPU submission, presentation,
or surrounding application work is included. Synthetic input time advances by 1/60 second;
the test does not sleep to simulate display cadence.

The report retains the drag-start frame, first movement, twenty subsequent movement
samples, release frame, and first following idle frame. Post-frame checks run outside the
timers: pointer movement changes only the selected source, topology and pan stay fixed,
490/1992 unrelated paths remain shared, and no frame has a routing fallback. Release
replaces every drag-history path; its geometry matches a forced cold rebuild. The following
idle frame reuses the release paths. The cold oracle runs after that idle frame so it does
not prime its allocator. No hardware-dependent timing assertion determines test success.

Native and Chrome release runs on the reference host on 2026-09-05, with unchanged production
code from `12988b8e` and the extended fixture, give the following observations. Complete
native/browser reports, host/browser identity, source context, and wasm hash are retained in
[`node_graph_drag_frames_native_browser.json`](../../benchmarks/performance/node_graph_drag_frames_native_browser.json).

| Target / fixture | Drag UI p95 | Tessellation p95 | Drag CPU frame p95 | Release frame (one sample) | Following idle (one sample) |
| --- | --- | --- | --- | --- | --- |
| Native, 100 nodes / 500 connections | 2.125 ms | 0.702 ms | 2.781 ms | 5.336 ms | 6.210 ms |
| Chrome, 100 nodes / 500 connections | 2.945 ms | 0.865 ms | 3.810 ms | 8.650 ms | 9.880 ms |
| Native, 500 nodes / 2000 connections | 8.334 ms | 1.374 ms | 9.659 ms | 34.628 ms | 26.924 ms |
| Chrome, 500 nodes / 2000 connections | 13.145 ms | 2.020 ms | 15.130 ms | 53.400 ms | 50.680 ms |

Component p95 values need not add to frame p95 because their nearest-rank samples can differ.
The existing drag rendering path skips individual response allocation and z-order raising;
release restores idle rendering and performs drop handling plus the full routing quality
rebuild. The following idle frame restores full response allocation. Thus fast moving frames
do not establish cheap release or idle behavior. The measurements prioritize release/idle
processing for further profiling; single release/idle samples do not establish their p95.
Both targets retain zero isolated, moving-frame, and release routing fallbacks. GPU and
full-application measurements, broader drag layouts, and frame-cost reduction remain open.

### Eligibility-first connected-node drops

The wire-splice owner rejects an already-connected node before building candidate-excluded
drop routing. Drag and placement confirmation call this shared operation. Such a node cannot
splice under the existing topology rules; its ordinary release-quality rebuild still runs.
Eligible unconnected drops continue to build current geometry after the final pointer event,
with only the candidate excluded. Pointer-less drops retain node-center targeting.

This removes an unused layout construction, routing-key change, and full routing pass from
connected-node release. It does not remove the final quality pass, change routing budgets,
alter frame membership, or optimize the following idle response-allocation pass. Tests verify
unchanged document/undo state and an exact routing-cache hit after rejecting input-connected
and output-connected candidates, for drag and placement. Eligible-drop tests cover final-event
movement, pointer/center targets, and checked routes after insertion. The full pointer-driven
fixture still requires every release path to be rebuilt and match the cold oracle.

Sequential native and Chrome release runs on the reference host on 2026-09-05 compare
`62a90857` with this change using the unchanged pointer-driven fixture. Source stays fixed
through every build/run and there is no concurrent cargo validation. Complete reports are in
[`node_graph_connected_drop_native_browser.json`](../../benchmarks/performance/node_graph_connected_drop_native_browser.json).

| Target / fixture | Release CPU frame before / after | Following idle before / after |
| --- | --- | --- |
| Native, 100 nodes / 500 connections | 5.234 / 3.843 ms | 5.952 / 6.317 ms |
| Chrome, 100 nodes / 500 connections | 8.580 / 6.245 ms | 9.645 / 10.215 ms |
| Native, 500 nodes / 2000 connections | 36.632 / 21.546 ms | 29.042 / 32.022 ms |
| Chrome, 500 nodes / 2000 connections | 49.475 / 32.415 ms | 46.415 / 51.600 ms |

Release and following-idle entries are one sample per fixture/run, not p95 or randomized
paired measurements. They show a lower observed release cost without establishing an idle
improvement; idle samples are higher in these runs. All isolated and pointer-driven drag
outcomes retain zero fallbacks. Remaining cold-routing and frame costs, broader layouts, and
GPU/application timing remain open acceptance work.

### Repeated release and idle frames

`paired-grid-pointer-release-cycles-v1` appends twenty warm alternating gestures to each
pointer-driven release-build fixture (two in debug tests). The same widget/context stays warm
after the separately reported long gesture. Each cycle establishes hover, presses the header,
crosses the drag threshold, and moves the connected node by +20 or −20 canvas-y units before
measuring release and its immediately following idle frame. Preparation frames are excluded.
The viewport is 1440 × 900, zoom is 0.35, and pan is [150, 100].

Every cycle checks complete release rebuilding, idle path-identity reuse, unchanged topology,
fixed other-node positions, no auto-pan, and zero fallbacks. Cached layout checks occur outside
the timers; the cold-route oracle runs only after both measured frames. `cycles.outcomes`
retains chronological frame timings and movement direction, while the distributions retain
sorted samples. The browser runner rejects incomplete cycles, invalid outcomes, nonfinite
timings, and distributions that disagree with their raw frames. Existing long-gesture single
release/idle measurements remain separate and are not pooled into these distributions.

Reference-host measurements on 2026-09-05, in CPU widget-plus-tessellation milliseconds:

| Host | Nodes / connections | Release p50 / p95 | Following idle p50 / p95 |
| --- | --- | --- | --- |
| Native | 100 / 500 | 3.58 / 4.40 | 6.19 / 6.54 |
| Chrome | 100 / 500 | 6.39 / 6.58 | 10.99 / 11.30 |
| Native | 500 / 2000 | 22.02 / 22.81 | 30.69 / 33.23 |
| Chrome | 500 / 2000 | 33.28 / 34.36 | 52.73 / 54.31 |

All cycles retain zero fallbacks. These are nearest-rank percentiles from twenty observations
per phase, not population-tail guarantees or a before/after speedup claim. Texture uploads,
GPU submission/presentation, and full application frames are excluded. Source stays fixed and
no Cargo validation runs concurrently with either measurement. Native execution is followed
by the isolated Chrome runner with its 45-second deadline and no virtual-time budget.
[`node_graph_release_cycles_native_browser.json`](../../benchmarks/performance/node_graph_release_cycles_native_browser.json)
contains both complete reports, per-cycle samples, hardware/browser metadata, and the wasm hash.

### Native application UI frame observation

The native application's `developer-tools` feature exposes `profile-frames`, an explicit
stationary UI measurement command. It displays a supplied graph through the existing
`App::new_with_graph` constructor in an actual WGPU-rendered window. It uses unavailable
execution/host services, not the normal native service bootstrap: no acquisition, graph
worker, native catalog, or user artifact repository participates. Graph-provided synthetic
previews can still appear. Eframe storage uses a fresh temporary directory, application save
is not delegated, and pointer/keyboard/drop input is suppressed. The input graph is never
written. This is a controlled application UI baseline, not a live processing baseline.

Build first, then run without concurrent Cargo validation:

```sh
cargo build -p logic-analyzer-app-native --release --features developer-tools --bin logic-conduit
node scripts/measure_application_frames.mjs target/release/logic-conduit \
  graphs/event_controls_demo.json --warmup 30 --samples 120
```

The Node 22+ runner supports macOS/Linux. `APP_FRAME_TIMEOUT_SECONDS` defaults to 45 seconds
and bounds the owned process group independently of the UI event loop. Timeout, interruption,
early close, invalid observations, or incomplete output fails the run; a plausible report
alone is insufficient without successful process completion. Cleanup targets only the process
group it starts. Regression tests cover report consistency, unsuccessful exits, and timeout
cleanup including a descendant: `node --test scripts/measure_application_frames_test.mjs`.

The window is non-resizable at 1440 × 900 logical points; the report records actual dimensions,
pixels per point, adapter, and surface configuration. Pan, zoom, and panels follow the graph's
restored document state. Thirty valid CPU observations warm the UI before 120 samples are
collected; egui multipass UI does not duplicate a rendered-frame observation. Changes to the
viewport or rendering configuration during sampling invalidate the run. `--screenshot` takes
a PNG only after all timing samples and requires a new output file, never overwriting one.
The process closes automatically after sampling and any requested screenshot.

`--minimum-seconds` (0–300, default 0) keeps the isolated UI rendering until both the requested
samples and the minimum lifetime are complete. Extra UI frames do not extend the sample set;
an early close fails even if the sample count is complete. Post-warmup surface-acquisition
failures invalidate the run while preserving the renderer's default recovery action. This
prevents CPU-only work on an occluded surface from masquerading as rendered-frame evidence.
`--always-on-top` explicitly activates the temporary profiling window and requests a window
level above normal windows. It is off by default, does not affect normal application windows,
and is recorded in each report. It does not bypass surface validation or guarantee visibility
on a locked desktop or behind higher-level system UI.

`eframe_cpu` is the framework's previous-frame CPU-side elapsed time, including UI and rendering
work but excluding its measured vsync wait. It is not GPU execution time. `ui_start_interval`
measures elapsed wall time between consecutive first UI passes, including pacing and host
scheduling; it is not a GPU duration or presentation-latency measurement. Chronological
`frames` records the UI frame at which that previous CPU value is observed. Sorted distributions
and nearest-rank p50/p95/p99/max are checked against all raw samples by the runner. Input-node
and connection counts identify the document, not independent post-load route/fallback checks.

The retained native reports use the reference M1 Ultra host with Metal, 2 pixels per point
(2880 × 1800 pixels), `AutoVsync`, and requested maximum frame latency 1. Both post-measurement
screenshots show routed nodes at restored 100% zoom, surrounding application panels, and
disabled Run/Start controls. Some graph content is offscreen in each restored viewport.
The event-controls document shows its synthetic waveform preview; the controlled-decode
document shows an empty capture and decoder tables, not a loaded file or executed pipeline.
Raw observations and source/binary hashes are retained in
[`node_graph_native_application_ui_frames.json`](../../benchmarks/performance/node_graph_native_application_ui_frames.json).

Final-executable observations on 2026-09-05, in milliseconds:

| Bundled document | Input nodes / connections | Eframe CPU p50 / p95 | UI-start interval p50 / p95 |
| --- | --- | --- | --- |
| `event_controls_demo.json` | 6 / 7 | 4.23 / 6.35 | 16.76 / 19.62 |
| `spi_controlled_decode.json` | 13 / 29 | 3.37 / 4.36 | 16.68 / 18.64 |

These are host-sensitive single-run observations, not stable performance thresholds; earlier
smoke captures are not pooled with the final-executable samples.

These small stationary documents do not establish large-graph scale acceptance, a routing
speedup, or full live-application performance. GPU upload/execution/presentation attribution,
large application graphs, application drag frames, browser rendering, and active-runtime
composition remain open work in step 6.

### Native Metal capture validation

`scripts/measure_application_metal.mjs` is an explicit macOS/Xcode capture runner, separate
from the CPU-only widget benchmarks. It launches only the isolated `profile-frames` app,
defaults to 30 warmup / 120 CPU samples and a 20-second minimum lifetime, then attaches
`Metal System Trace` to that PID after three seconds for a five-second recording. A separate
45-second watchdog bounds application and profiler together. The app must remain alive until
the recording time-limit marker; saving the trace may finish after the app. Both processes
must exit successfully and the CPU report must validate. Existing trace paths are rejected;
cleanup kills only owned process groups. Capture completion alone is not rendering acceptance.

```sh
node scripts/measure_application_metal.mjs target/release/logic-conduit <fanout.json> <new.trace> --always-on-top
xcrun xctrace export --input <new.trace> \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="metal-gpu-intervals"]' --output <gpu.xml>
xcrun xctrace export --input <new.trace> \
  --xpath '/trace-toc/run[@number="1"]/data/table[@schema="metal-application-intervals"]' --output <app.xml>
python3 -B scripts/summarize_metal_trace.py <gpu.xml> <app.xml> --pid <owned-pid> --require-label egui_render
```

The Python-stdlib summarizer resolves document-local XML identities, filters exact process
ownership, and joins top-level active GPU intervals to typed command-buffer encoding labels.
It analyzes the preselected trace-relative 1–4 second window and excludes entire buffers
crossing either boundary. Per-buffer span includes stage gaps; active union merges overlapping
intervals instead of adding them. Apple GPU stages can execute concurrently, so summed stage
durations are not elapsed frame time; see [Apple's GPU timeline documentation](https://developer.apple.com/documentation/xcode/analyzing-apple-gpu-performance-using-a-visual-timeline).
Neither metric includes CPU upload/staging cost or establishes presentation latency. The
required `egui_render` label prevents write-only GPU activity from passing rendering acceptance.
Raw Instruments exports can include unrelated system metadata even with `--attach`; only
process-filtered evidence belongs in the repository.

The occluded reference-host 2026-09-06 scale captures complete but fail rendering acceptance: the
100-node analysis window contains 943 pending-write and 259 signal buffers; the 500-node
window contains 206 and 58 respectively, with no `egui_render` buffers in either. Hardened
reruns explicitly report surface acquisition `Occluded` at both scales. Their CPU/cadence
values and write-only GPU durations are rejected as application rendering baselines, not
treated as improvements. Source identities, complete rejected CPU reports, filtered summary
distributions and guard failures are in
[`node_graph_metal_capture_validation.json`](../../benchmarks/performance/node_graph_metal_capture_validation.json).

Regression checks run with `node --test scripts/measure_application_metal_test.mjs` and
`python3 -B scripts/summarize_metal_trace_test.py`. They cover target liveness versus save time,
failure/timeout cleanup including descendants, XML references, process isolation, overlapping
and nested intervals, boundary exclusions, missing required render work and malformed data.

### Visible native GPU observations

The opt-in foreground setting supports visible-window measurement without relaxing the
surface-acquisition guard. On the reference M1 Ultra host on 2026-09-06, both application
scale fixtures complete the bounded capture with `--always-on-top` and a post-measurement
screenshot. The trace-relative 1–4 second window contains 170 and 175 `egui_render` command
buffers respectively. Each figure below is a single-run nearest-rank observation, not a
before/after comparison, confidence interval, or full application-frame measurement.

| Nodes / connections | Render GPU span p50 / p95 | Render GPU active union p50 / p95 | Pending-write GPU span p50 / p95 |
| --- | --- | --- | --- |
| 100 / 500 | 0.225 / 0.230 ms | 0.207 / 0.210 ms | 0.014 / 0.045 ms |
| 500 / 2000 | 0.222 / 0.228 ms | 0.205 / 0.208 ms | 0.014 / 0.047 ms |

Separate uninstrumented CPU runs, each with 30 warmup / 120 observations and the same foreground
setting, show eframe CPU p50/p95 of 3.27/3.61 ms and 12.53/13.42 ms respectively. CPU and GPU
samples cover different windows and are not added or subtracted. Instrumented CPU observations
remain in the artifact as capture-completion evidence, not an uninstrumented timing baseline.

All four post-measurement screenshots are inspected. CPU and trace screenshots have identical
hashes at each scale: 100% graph zoom, default split panels, empty capture/viewer, disabled
Run/Start controls, visible fan-out, and 100/500 node counts with most content offscreen. No
migration/error toasts appear. The larger fixture shows more angular routes; appearance does
not establish route quality or fallback counts. This workload does not represent the neutral
low-zoom full CPU widget fixture or a fully visible large graph. Its similar GPU spans therefore
do not establish constant rendering cost with graph size or a whole-graph 60 Hz guarantee.

Complete process-filtered in-window command-buffer intervals, distributions, excluded counts,
uninstrumented and instrumented CPU reports, binary/XML identities and screenshot checks are in
[`node_graph_visible_native_gpu.json`](../../benchmarks/performance/node_graph_visible_native_gpu.json).
Source and executable stay fixed during capture; Cargo validation does not run concurrently.

#### Proposed future rendering validation

Measure CPU upload/staging and frame-to-presentation latency separately from command-buffer
GPU execution. Full application/active-runtime composition, browser rendering, low-zoom
application/drag scenarios and release/idle tails remain separate acceptance work.

### Native application UI scale documents

The explicit `application-frame-fixture` example generates `paired-builtins-fanout-v1`:
100 nodes/500 wires or 500 nodes/2000 wires, using the existing logic transform and parallel
decoder records from `graphs/wasm_decoder_demo.json`. It introduces no new node definitions,
application API, or production routing behavior. Each pair connects the transform's
single output to ten decoder inputs at the smaller scale and eight at the larger scale.
Five pairs occupy each row, with pair origins 900 canvas units apart horizontally and 700
vertically; the decoder is 450 units right of its transform. Real node controls, socket
counts, and shared-output fan-out differ from the neutral `paired-grid-v1` benchmark.

The generated document contains no capture sources, copied panel layouts, or copied payload
subscriptions. It explicitly deselects each output using the current viewer-selection document
contract. The surrounding viewer is empty, with no graph execution or acquisition. The native
profile uses its fresh default split-panel layout and 100% graph zoom, not the neutral CPU
fixture's 35% zoom. Most nodes are offscreen; this is stationary default-view coverage, not a
full-visible-graph or low-zoom application benchmark.

The generator restores the template through the registered editor definitions and checks exact
wire preservation, node counts, and absence of node validation badges after loading the result.
Its ordinary bounded tests cover both sizes, deterministic JSON bytes, directed fan-out endpoints,
positions, the next allocation ID, and complete serialized equality after reloading. Long frame
measurement remains an explicit command. Output files require a new path and are never overwritten.

Build both executables before measuring; generate each fixture into a fresh temporary directory:

```sh
cargo build -p logic-analyzer-examples --release --example application-frame-fixture
cargo build -p logic-analyzer-app-native --release --features developer-tools --bin logic-conduit
frame_fixture_dir=$(mktemp -d /tmp/node-graph-app-scale.XXXXXX)
target/release/examples/application-frame-fixture "$frame_fixture_dir/fanout-100.json" --nodes 100
target/release/examples/application-frame-fixture "$frame_fixture_dir/fanout-500.json" --nodes 500
node scripts/measure_application_frames.mjs target/release/logic-conduit \
  "$frame_fixture_dir/fanout-100.json" --warmup 30 --samples 120
node scripts/measure_application_frames.mjs target/release/logic-conduit \
  "$frame_fixture_dir/fanout-500.json" --warmup 30 --samples 120
```

Reference-host observations on 2026-09-05 use the same Metal adapter, 1440 × 900 logical
viewport, 2 pixels per point, and `AutoVsync` configuration as the small-document baseline.
Both 120-sample runs complete within the runner's 45-second deadline. No Cargo validation
runs concurrently and source stays fixed throughout measurement. Post-measurement screenshots
show the expected node counts and fan-out, with no source-conflict or migration toasts.

| Nodes / connections | Eframe CPU p50 / p95 | UI-start interval p50 / p95 | Maximum UI-start interval |
| --- | --- | --- | --- |
| 100 / 500 | 3.34 / 3.58 ms | 16.67 / 16.91 ms | 83.56 ms |
| 500 / 2000 | 13.39 / 13.85 ms | 16.67 / 16.87 ms | 33.22 ms |

The raw report retains every interval, including host-sensitive outliers; p95 alone does not
establish consistently smooth presentation. These are neither speedup ratios nor GPU durations.
The screenshot at the larger size shows more angular routes; the smaller fan-out includes short
detours. Appearance does not identify fallback reasons or prove routing quality, and this
application harness does not expose route outcome counts. Route collision/fallback/quality
acceptance remains separate from these UI timing observations. Raw samples, generator metadata,
document hashes, and source/binary/visual-check hashes are in
[`node_graph_native_application_scale_frames.json`](../../benchmarks/performance/node_graph_native_application_scale_frames.json).

### Native idle-frame sampling

The `idle_profile` example is an explicit profiling command: it runs 1000 stationary
CPU widget/tessellation frames at 500 nodes / 2000 connections, zoom 0.35, and 1440 × 900.
It constructs paired-grid-v1 through the widget's public API, establishes zoom through input
events, and clears texture uploads like the other CPU harnesses. It checks nonempty meshes
and unchanged graph state at completion; route correctness is covered separately by the
routing tests. The optional positive frame count defaults to 1000. Normal tests and timed
release reports retain their existing bounded sample counts; the profiling workload is opt-in.

Build and launch it with:

```sh
cargo build -p node-graph --release --example idle_profile
target/release/examples/idle_profile 1000
```

Attach an external sampling profiler to the example executable, not Cargo. On macOS, run
`sample <example-process-pid> 3 1 -file /path/to/profile.txt` in another terminal. Start sampling
after fixture construction. The workload itself is portable Rust; the external profiler is
a host choice and introduces no target-dependent widget source or dependency.

A three-second native sample on the reference M1 Ultra host on 2026-09-05, roughly seven
seconds after launch, captures 2336 main-thread stacks. Three call sites that resolve to
`raise_node_hit_targets` account for 1619 of them (69.3%), predominantly in calls to
`egui::WidgetRects::insert` and list movement. This is a lower bound: smaller raising branches
are not included. The initial response-allocation pass is distinct from these repeated
z-order updates and excluded from that sum. Optimized functions are inlined into
`show_with_panel_data`; `dsymutil` and `atos -inlineFrames` recover their source attribution.
Source and settings stay fixed through the run; no cargo validation runs concurrently.
The command completes all 1000 frames successfully. Raw header/call-graph evidence,
selected inline-symbolication output, and aggregation metadata are in
[`node_graph_idle_native_sample.txt`](../../benchmarks/performance/node_graph_idle_native_sample.txt)
and [`node_graph_idle_native_sample.json`](../../benchmarks/performance/node_graph_idle_native_sample.json).

The locked egui 0.36.1 implementation removes a raised widget from its layer vector, appends
it, and updates the shifted widgets' stored indices. Repeated raises therefore move and
reindex many unrelated targets even though offscreen targets skip their own raises. This
profile identifies a native CPU hotspot; 69.3% is neither a predicted speedup nor browser
attribution, frame p95, or GPU/application time. Production interaction behavior is unchanged.

### Native release-cycle sampling

The explicit command `idle_profile 200 release` runs 200 alternating pointer-driven gestures
through the widget's public API. Each cycle contains four preparation frames, the release
frame, and its following idle frame. Non-inlined `prepare_release`, `release_frame`, and
`settled_frame` wrappers keep those phases distinguishable in optimized stack samples without
adding hooks to production rendering. The original `idle_profile [count]` idle mode remains
the default. Argument and two-cycle interaction tests run with
`cargo test -p node-graph --example idle_profile`.

This sampling fixture uses 500 nodes / 2000 connections, 35% zoom, and a 1440 × 900 CPU
viewport. Its canvas grid is translated by `(150, 100) / 0.35`, keeping the header grip away
from auto-pan without exposing a private view setter. It is a separate sampling workload,
not a replacement or pooled timing sample for `paired-grid-v1`. Every cycle checks drag
activation, exact displacement, release/idle state, and stable final position. Final document
comparison permits only selection and the moved node's position; other graph state and zoom
remain unchanged. Every frame requires nonempty meshes. Private route identity and cold-oracle
checks remain in the ordinary routing regressions, outside this public-API profiling command.

Build the optimized example, run it, and attach `sample <its-pid> 5 1 -file <output.txt>` after
the readiness message. On the reference M1 Ultra host on 2026-09-06, a five-second sample
captures 3453 main-thread stacks: 2094 in preparation, 617 in release, 740 in following idle,
and two unclassified. The command completes all 200 cycles under a 45-second watchdog. Cargo
validation does not run concurrently; `dsymutil` and `atos -inlineFrames` provide attribution.

| Sampled phase | Selected inclusive branch | Samples / phase | Interpretation |
| --- | --- | --- | --- |
| Release | Routing-cache rebuild | 347 / 617 (56.2%) | Lower bound from one rebuild branch |
| Release | Candidate-band float sorting | 109 / 617 (17.7%) | Subset of the rebuild branch, not additive |
| Following idle | Socket hit-target refresh | 340 / 740 (45.9%) | Lower bound; predominantly egui list moves and index maintenance |

The sorting stack is inside `route_bundle`, where candidate bands are ordered by distance
from the ideal band and then by their numeric coordinate. The idle stack
still reaches `WidgetRects::insert` through repeated hit-target refreshes. These observations
identify CPU optimization candidates; they do not explain the previous native timing outlier,
measure tail latency, predict a speedup, or include browser/GPU/full-application work.
Raw call-graph evidence, selected inline attribution, executable identity, and methodology are
in [`node_graph_release_native_sample.txt`](../../benchmarks/performance/node_graph_release_native_sample.txt)
and [`node_graph_release_native_sample.json`](../../benchmarks/performance/node_graph_release_native_sample.json).

### Candidate-band ordering

The bundle router numerically sorts and deduplicates finite band coordinates before sorting
by proximity. Aligned obstacle rows therefore contribute only one copy of each coordinate to
the more expensive distance sort. Both sorts are in-place unstable sorts. The original `f64`
distance comparator and total coordinate tie-breaker define the final order, including the
negative-zero representative when both zero signs occur. Coordinate generation is charged
before deduplication; candidate iteration and work-budget spending remain unchanged.
Bitwise comparisons against the original stable-sort/dedup oracle cover float boundaries,
distance ties, duplicated and unique coordinates, and reversed, sorted, and shuffled inputs.

Paired release builds on the reference M1 Ultra host and Chrome 152 on 2026-09-06 compare
this ordering with the original stable distance sort at `39825a81`. Each platform has three
process pairs with both baseline-first and candidate-first order, fixed before measurement.
Each invocation uses the unchanged two-scale CPU fixtures and twenty samples per distribution;
every browser run has a fresh isolated profile. Cargo validation does not run concurrently.
The table shows medians of the three per-run p50/p95 values in milliseconds, baseline → current.

| Platform / nodes / connections | Cold routing p50 | Release CPU p50 | Release CPU p95 | Following-idle CPU p95 |
| --- | --- | --- | --- | --- |
| Native / 100 / 500 | 1.16 → 1.11 | 3.54 → 3.49 | 3.90 → 3.75 | 5.83 → 5.82 |
| Native / 500 / 2000 | 12.28 → 10.81 | 22.24 → 20.51 | 23.05 → 21.30 | 27.55 → 26.98 |
| Chrome / 100 / 500 | 2.37 → 2.32 | 5.90 → 5.83 | 6.10 → 6.05 | 9.10 → 8.95 |
| Chrome / 500 / 2000 | 17.35 → 15.23 | 31.87 → 29.93 | 32.64 → 30.82 | 43.19 → 42.63 |

For the large fixture, cold routing improves about 12% on both platforms; release p50 improves
7.8% on native and 6.1% in Chrome. All three pairs improve large-fixture cold routing and release
p50/p95 on both platforms. Every retained run passes the cold/history-aware checks and has zero
cold/release/idle fallbacks with the existing budgets. A sort-algorithm-only comparison, without
early deduplication, shows no consistent material native improvement and is not the selected
implementation. Its complete paired reports are retained separately within the same
[`node_graph_band_order_native_browser.json`](../../benchmarks/performance/node_graph_band_order_native_browser.json)
artifact, alongside binary identities, schedules, browser versions, all samples, and collection notes.

These are descriptive CPU results for aligned-row fixtures, not confidence intervals, a
tail-latency guarantee, or proof of gains for arbitrary unique-band layouts. Stationary and
following-idle costs are not the target of this change. Large-graph frames remain above
16.67 ms, and these measurements do not include GPU submission or full application composition.

### Partial hit-target move elision

The response/render owners refresh every initially registered target with its original ID,
sense, and focus-call order. Below 60% zoom, when inline controls are absent, redundant list
moves are omitted only for fully clipped-in nodes whose target bounds, expanded by twice the
global near-hit radius, do not intersect another node's targets. Clipped or overlapping nodes,
transformed layers, invalid radii, and geometry/topology changes between allocation and drawing
use full raising. Any change in the overall target identities or minimap visibility also
disables elision: deleted targets can remain registered for the rest of the input frame.
Routing geometry and work budgets are independent of this optimization.

For an eligible node, the move plan reconstructs initial socket ranks from the allocation
layout and walks the desired body/header/toggle/socket order. A target moves only when an
earlier potentially competing target currently ranks above it. Separated targets cannot share
a direct or near hit, so their relative ranks need not match. Initial and drawing socket-map
orders can differ; their ranks are compared explicitly. Fast dragging skips plan construction.
A release frame with canvas-only initial allocation also skips planning: its node targets are
new insertions, not repeated moves.

Native regressions check direct and near hits against full raising, all permutations of five
abstract ranks, overlapping sockets and toggles, floating overlays, clipped-target Tab/Shift-Tab
order, capture through geometry changes, and focus across the inline-control zoom threshold.
Eligibility checks assert that moves are actually omitted for safe targets and that clipping,
transforms, invalid zoom/radius, changed geometry, changed topology, and overlapping bounds
fall back. Initial registrations remain intact because clipped interactive targets still
participate in keyboard focus.

On the reference M1 Ultra host on 2026-09-06, three pairs per platform compare the unchanged
neutral 35%-zoom fixtures against full raising. Baseline/candidate order is randomized within
each pair before execution. Each run uses an immutable copied release executable or wasm
artifact; browser runs launch fresh isolated Chrome 152 profiles. Cargo validation does not
run concurrently with measurements. Each process retains twenty warm stationary frames and
twenty pointer-driven release/idle cycles per size, plus moving frames and first-frame costs.

The table reports medians of three per-run medians or p95s, not pooled percentiles or confidence
intervals. All runs preserve the existing topology, cache/rebuild, cold-oracle, and zero-fallback
assertions.

| Platform | Nodes / connections | Stationary frame median, baseline → elision | Post-release idle p95, baseline → elision |
| --- | --- | --- | --- |
| Native | 100 / 500 | 7.86 → 7.34 ms | 7.31 → 5.88 ms |
| Native | 500 / 2000 | 36.05 → 32.95 ms | 32.60 → 32.53 ms |
| Chrome | 100 / 500 | 12.51 → 11.42 ms | 10.51 → 9.05 ms |
| Chrome | 500 / 2000 | 59.40 → 51.17 ms | 50.21 → 43.88 ms |

The large stationary median improves by about 8.6% native and 13.9% browser in these trials.
Moving and release frames do not establish a consistent speedup. Native large release-frame
p95 ranges are 22.89–23.48 ms baseline and 23.08–32.02 ms with elision; browser ranges are
32.66–33.14 and 32.32–33.05 ms. Native large idle-p95 ranges also overlap substantially
(32.40–35.99 versus 30.16–38.96 ms), without a meaningful median improvement. The slow native
candidate run remains in the report; native upper tails therefore still warrant profiling.
Large stationary frames remain above a 60 Hz budget. These CPU widget/tessellation fixtures
do not measure GPU upload, execution, presentation, full application composition, or active
runtime load.

Full distributions, binary hashes, chronological pair order, and browser metadata are in
[`node_graph_hit_move_elision_paired_native.json`](../../benchmarks/performance/node_graph_hit_move_elision_paired_native.json)
and [`node_graph_hit_move_elision_browser.json`](../../benchmarks/performance/node_graph_hit_move_elision_browser.json).
The browser artifact keeps preliminary variants separate from the final paired summary.

#### Proposed future interaction-order work

Dependency-level ordered registration or batched updates may reduce the remaining list-move
cost, but require a separate API review. Any such approach must preserve painted overlap
priority, socket order, floating overlays, protruding hit areas, capture, focus, and same-frame
geometry changes. GPU/application profiling and remaining release/idle CPU costs are tracked
in `TODO.md`.

## Reference workloads

All performance claims are measured on two reference captures. A change is not accepted on the
evidence of one of them.

| Workload | Packed input | Waveform index blocks | Derived words | Role |
| --- | --- | --- | --- | --- |
| `scan.dsl` | 1.246 GB | 605 | 136,939,197 | Smaller reference; sensitive to fixed overhead and publication latency |
| Larger reference capture | 2.729 GB | 1,309 | 171,356,637 | Scaling reference; sensitive to per-block and per-batch costs |

The end-to-end acceptance benchmark is:

```sh
logic-conduit run graphs/spi_controlled_decode.json --json
```

Its report keeps artifact count, stored bytes, execution time, CPU utilization, and
final-publication latency visible, so a change that trades one for another cannot be accepted
silently.

## Acceptance rule

Every performance change is compared against the retained baseline on both reference captures, and
must report:

1. exact output and artifact identities — output fingerprints, derived word counts, block counts,
   and stored bytes;
2. wall time and CPU time, distinguishing critical-path wall time from overlapping cumulative
   worker time;
3. peak resident memory;
4. cancellation bounds;
5. native and wasm behavior; and
6. concurrent viewer frame and query latency at p50/p95/p99.

**A throughput improvement that harms foreground responsiveness is rejected.** This rule has
teeth: it is the reason the runtime does not use the fastest measured scheduling policy (see
[Idle backoff](#idle-backoff-and-the-benchmarkproduction-discrepancy)).

Two failure modes this rule exists to catch have both occurred in practice:

- **Microbenchmark-only improvements.** A change that lowers one worker's cumulative CPU while
  raising wall time is a regression. Cumulative CPU across overlapping workers is not a
  critical-path measurement.
- **Benchmark-only improvements.** A probe harness that omits production scheduling policy can
  report a wall time the application never achieves.

## Performance-relevant architecture

These are design commitments, not tuning parameters. Each is the durable form of a measured result.

### Reproducible regression comparisons

The `performance-regression` binary in `logic-analyzer-examples` owns opt-in end-to-end baseline
recording and comparison. A workload JSON document identifies the graph, working directory,
capture-path injection points, warmup count, measured-run count, and metrics allowed to justify a
retention decision. Large captures remain outside the repository; `--capture` or the workload's
named environment variable supplies one. The runner fingerprints the complete capture before the
warmups and retains its byte length and canonical path as diagnostic metadata.

Each measured invocation runs `logic-conduit run <temporary-graph> --json --progress-interval 0`.
Unix process accounting supplies wall time, user-plus-system CPU time, and peak resident memory.
The application report supplies execution time, exact item/block/byte counts, cache identities,
and fingerprints of finalized derived data. A mismatch between runs, the retained baseline, or an
A/B reference is a hard failure.

Recording a baseline and comparing a candidate use the checked-in reference workload as follows:

```sh
cargo build --release -p logic-analyzer-app-native -p logic-analyzer-examples \
  --bin logic-conduit --bin performance-regression

target/release/performance-regression record \
  --workload benchmarks/performance/spi_controlled_decode.json \
  --capture /path/to/reference.dsl \
  --binary target/release/logic-conduit \
  --baseline /path/to/baseline.json

target/release/performance-regression compare \
  --workload benchmarks/performance/spi_controlled_decode.json \
  --capture /path/to/reference.dsl \
  --baseline /path/to/baseline.json \
  --candidate /path/to/candidate/logic-conduit \
  --reference /path/to/reference/logic-conduit \
  --output /path/to/comparison.json
```

When both executables are supplied, their order reverses for every warmup and measured pair. Every
metric reports its median, minimum, maximum, and spread. A configured acceptance metric counts as
improved only when the candidate's complete range is below the reference's complete range; overlap
is inconclusive and cannot produce `retain`. A non-overlapping CPU, peak-RSS, or execution-time
regression rejects the candidate as a guardrail. Viewer p50/p95/p99 latency remains the documented
manual concurrent-viewer step and is called out in every baseline and comparison report.

### Segment artifacts, not per-item files

Both the waveform index and the derived-word store publish a bounded number of large immutable
segment artifacts rather than one file per leaf or block. Per-artifact filesystem publication —
create, write, truncate, rename, and durability barriers — dominated both pipelines before this
change and scaled with artifact count rather than with data volume.

- The waveform index groups 64 channel-major leaves per segment, retains a four-segment
  immutable-region cache, and publishes the root last.
- The derived store appends encoded blocks into segment-sized writable mappings or buffered
  regions, publishes only complete segments plus the final index/manifest generation, and relies on
  ordinary OS page-cache writeback rather than a durability barrier per block. Live queries are
  preserved through a bounded in-memory view of blocks in the unpublished active segment.

Both formats reject their pre-segment version so a stale cache rebuilds automatically rather than
being read through a compatibility path.

### Bounded pipelining through the injected executor

Source reading, CPU summary work, and artifact writing are pipelined through the host executor with
bounded in-flight work. Each bounded worker owns one source reader; local workers share `BlockData`
backing so no handoff copy is required; the coordinator publishes completed leaves in per-channel
order.

Worker-count scaling is measured, not assumed. Finite index builds cap their bounded worker pool at
**12**: post-segmentation sweeps peak at 12 workers on both captures and regress at 16–20. This cap
is specific to finite index builds and is deliberately not imposed as a generic executor limit.

### Idle backoff and the benchmark/production discrepancy

The threaded manager honors `WorkOutcome::made_progress`, briefly yields through the injected
executor, and then applies a **50 µs** idle backoff. This replaced a fixed 2 ms idle delay.

The two policies previously measured on `scan.dsl` show why both wall time and CPU must be
reported:

| Policy | Wall time | CPU time |
| --- | --- | --- |
| No-backoff probe harness | 2.86 s | 21.73 CPU-s |
| Fixed 2 ms idle delay (then production) | 24.62 s | 16.69 CPU-s |
| `made_progress` + yield + 50 µs backoff (retained) | 2.70 s | 15.32 CPU-s |

The no-backoff probe was the benchmark the optimization work had been using; the 2 ms delay was
what the application actually ran. Neither number described the other. The retained policy is
better than both on both axes, and completed the larger capture in 3.56 s using 19.92 CPU-seconds
with unchanged output fingerprints.

### Foreground responsiveness budget

The viewer is measured **while** a durable cache workload runs, not in isolation. The budget is an
8 ms frame; the retained baseline has never exceeded it in these runs.

Measured across the retained changes, in order:

| After | Pointer-input frame p50/p95/p99 | Lane query p50/p95/p99 | Frames over 8 ms |
| --- | --- | --- | --- |
| Scheduling backoff | 0.50 / 1.01 / 1.05 ms | 0.41 / 0.54 / 0.67 ms | none |
| Sampling publication | 0.50 / 1.02 / 1.18 ms | 0.39 / 0.55 / 0.68 ms | none |
| Output coalescing | — / — / 1.41 ms | — / — / 0.91 ms | none |
| Positional reads | — / — / 1.22 ms | — / — / 0.69 ms | none |

Throughput roughly doubled across these changes while foreground latency stayed flat, which is the
result the acceptance rule exists to protect.

### Platform boundaries stay put

Performance work does not relocate ownership:

- `signal_capture` owns the portable kernel contract and CPU fallback; `platform`
  owns native and web adapters, capability discovery, and unavailable-hardware handling.
- No target conditionals or GPU dependencies enter portable processing, viewer, compiler, or
  concrete-node crates.
- Cache identity never depends on the selected device or backend.
- Positional-read optimization lives in the platform adapter and one explicitly allowlisted
  processing file adapter, with a cursor fallback for non-Unix hosts.

### GPU acceleration is conditional and currently unjustified

GPU work is gated on a regular, batchable, transfer-efficient kernel remaining on the critical
path. No current profile shows one. Remaining CPU is distributed across source reading, ZIP
inflation, fragment scanning, ordered decoder work, variable-length derived encoding, and sinks.
Dispatching the packed-summary kernel would additionally transfer 1.25–2.73 GB of packed input for
work that is already off the critical path.

## Retained baseline

Current end-to-end durable-run numbers, after all retained changes:

| Measurement | `scan.dsl` | Larger capture |
| --- | --- | --- |
| Pipeline wall time | 2.25–2.27 s; positional reads left it within noise at 2.22–2.30 s | 2.91–2.96 s, from 2.96–3.00 s before positional reads |
| Peak RSS | 436–458 MB | 456–478 MB |
| Waveform index segments | 10 (was 605 leaf files) | 21 (was 1,309) |
| Derived segments | 82 (was 2,753 block files) | 99 (was 3,237) |
| Derived words | 136,939,197 | 171,356,637 |
| Derived blocks (Parallel Decoder lane) | 1,063 | 1,321 |
| Durable derived footprint | 188.8 MB | 235.8 MB |
| Output batches | 2,179 (from 13,830 fragment scans) | 2,728 (from 30,286) |

Concurrent viewer latency under the durable workload, most recent measurement:

| Measurement | Value |
| --- | --- |
| Pointer-input frame p99 | 1.22 ms |
| Lane query p99 | 0.69 ms |
| Frames over 8 ms | none |

Fixed bounds that are load-bearing and were each validated against an alternative:

- Fragment window: **65,536 samples**. Doubling it raised `scan.dsl` peak memory from 468 MB to
  536 MB for only a small wall-time gain. It also bounds cancellation latency.
- Output batch: **65,536 words**, the maximum output of one fragment. A 32,768-word probe increased
  send counts without improving wall time or peak memory.
- Derived block: **131,072 words**.
- Derived staging: **8 MiB per lane**. A 32 MiB prototype retained the filesystem gain but with a
  higher aggregate staging footprint across concurrent lanes.

## Investigation record

### Waveform index generation

**Profile.** The initial `scan.dsl` cold build attributed 1.61 s of its 1.66 s wall time to source
reading and decompression. Summary work consumed 0.38 cumulative CPU-seconds; packed-block copying
consumed about 22 ms and in-memory artifact publication about 15 ms. A second profile on 2.73 GB of
packed input completed in 1.71 s with 7.66 cumulative worker-seconds in reads and 0.73 in
summaries. Source reading and decompression were the critical path; the summary kernel was not.

**Retained — CPU path and copy elimination.** Removing avoidable packed-block copies and pipelining
bounded read, summary, and write work produced a 0.76 s median across five `scan.dsl` runs, against
1.66–1.69 s before: an approximately **2.2× median speedup** with zero handoff-copy time. Two and
four workers performed equivalently within about 1%, while 20 workers were 11–17% slower and
consumed more CPU.

**Retained — segment artifacts.** With the CPU path optimized, publication dominated: 605 leaf
files took about 3.4 s of a 3.44–4.04 s parallel build, and 1,309 leaf files took about 7.56 s of a
7.59–8.49 s build. Segmentation reduced this decisively:

| Capture | Artifacts | Publication | Best wall time |
| --- | --- | --- | --- |
| `scan.dsl` | 605 leaves → 10 segments | 3.4 s → 0.10 s | 3.44 s → 0.34 s |
| Larger | 1,309 leaves → 21 segments | 7.56 s → 0.18 s | 7.59 s → 0.60 s |

**Rejected — alternative inflate backends.** `zip` selects `flate2` with the pure-Rust `zlib-rs`
backend. An otherwise identical `miniz_oxide` build was slower on both captures, and a native
`zlib-ng` upper-bound probe offered no useful headroom.

| Backend | `scan.dsl` warm wall / read CPU | Larger warm wall / read CPU |
| --- | --- | --- |
| `zlib-rs` (retained) | 0.30–0.34 s / 1.84–1.91 CPU-s | 0.53–0.56 s / 2.60–2.82 CPU-s |
| `miniz_oxide` | 0.40–0.41 s / 2.82–2.89 CPU-s | 0.70 s / 4.19 CPU-s |
| `zlib-ng` (native probe) | slightly worse | wall overlaps; read CPU ≈ 2.80 s |

Every backend preserved 605/1,309 blocks and 1.246/2.729 GB of packed input. No injected native
decompression capability is justified. Further index acceleration must target archive-level work
scheduling or reuse, and must begin with evidence of duplicate decompression on a real critical
path rather than another codec swap.

**Archive work attribution.** The opt-in DSL probe groups archive, prepared-source, cache, and wait
counters by immutable source identity and by metadata, waveform-index, runtime-delivery, and
presentation-query phase. On `scan.dsl`, a cold indexed run followed by delivery of the first one
million samples records 607 index-phase decompressions (253,935,044 compressed bytes expanding to
1,246,200,448 bytes). Runtime delivery then performs another 11 decompressions for 18,875,048
expanded bytes. All 183,266 prepared-source bytes read by that runtime phase overlap ranges already
read earlier in the same source generation. Presentation setup separately rereads all 32,875 source
bytes it requests.

A controlled two-viewer test opens independent exact-query readers concurrently. Each reader
expands its own header and requested logic block, producing four presentation-phase decompressions
for two identical block requests. The runtime reader's local cache reports one miss followed by one
hit for the same block. The evidence therefore confirms useful expanded-block reuse across index,
runtime, and viewer readers; it supports evaluating the bounded shared cache next, without yet
changing archive ownership or scheduling.

### Derived-data storage

**Profile.** `scan.dsl` published 2,753 immutable block files containing 591 MB and spent 2.61
cumulative seconds in block create/write/truncate/rename calls during a 3.04 s pipeline; its 14
final index/manifest pairs consumed about 0.37 cumulative seconds including durability barriers.
The larger capture published 3,237 block files containing 741 MB, spending 3.02 cumulative seconds
during a 4.02 s pipeline, with final index/manifest publication consuming about 0.33 cumulative
seconds. Repository call times overlap, but artifact counts, system CPU, and scaling consistently
identified per-block filesystem publication as the bottleneck.

**Retained — segment artifacts.** With an 8 MiB per-lane staging target:

| Capture | Artifacts | Wall time | System CPU |
| --- | --- | --- | --- |
| `scan.dsl` | 2,753 blocks → 82 segments | 3.04 s → 2.92 s median | 3.67 s → 2.40 s median |
| Larger | 3,237 blocks → 99 segments | 4.02 s → 3.58–3.72 s | 5.10 s → 2.79–3.05 s |

Stored bytes, word counts, and both output fingerprints were unchanged. Per-lane index-to-manifest
publication spans, measured separately from overlapping cumulative repository-call time, were 78 ms
for `scan.dsl` and 44 ms for the larger capture at the slowest lane.

### Graph execution and the decoder critical path

**Retained — scheduling.** See [Idle backoff](#idle-backoff-and-the-benchmarkproduction-discrepancy).
Diagnostic-only executor labels separate DSL block reading, parallel fragment scans, and
derived-block encoding; sampled node metrics expose work-call count, wall latency, and thread CPU
without changing execution policy.

**Retained — sampling-point publication.** Completion waiting proved negligible; merge plus durable
sampling publication was the largest serialized section. The persistent sampling store now owns an
opaque, storage-ready word batch, so the decoder encodes directly into the queued writer's
representation instead of retaining a second `Vec<PackedSamplingPoint>` and converting it during
publication.

| Measurement | `scan.dsl` | Larger capture |
| --- | --- | --- |
| Sampling publication | 709 ms → 49 ms | 845 ms → 52 ms |
| Merge + publication | 2.05 s → 1.60 s | 2.46 s → 1.91 s |
| Pipeline wall | 2.70 s → 2.43 s | 3.56 s → 3.05 s |

**Retained — output coalescing.** The decoder merges into one reused ordered pending batch bounded
at 65,536 words and flushes a partial tail through the shared streamed lifecycle before
end-of-stream, without weakening channel backpressure.

| Measurement | `scan.dsl` | Larger capture |
| --- | --- | --- |
| Fragment scans → output batches | 13,830 → 2,179 | 30,286 → 2,728 |
| Output-send time | 327 ms → 230–242 ms | 378 ms → 260 ms |
| Retained-collector calls | 129,834 → 41,154 | 158,439 → 57,462 |
| File-writer calls | 8,425 → 2,108 | 10,634 → 2,635 |
| Wall time | 2.43 s → 2.25–2.27 s | 3.05 s → 2.96–3.00 s |

The largest observed pending batch was 64,900 words. The `scan.dsl` range excludes one 2.45 s cold
outlier; warm runs are the comparison basis throughout this document.

**Retained — native positional reads.** A refreshed native sample placed source reads and ZIP
inflation ahead of packed-summary work; seek calls were visible but not themselves dominant. Unix
readers in the platform adapter and the allowlisted processing adapter now use positional file
reads. Alternating exact-build A/B runs showed a small but repeatable effect on the larger
reference: warm waveform-index read work fell by about 2–4%, and durable runtime wall time moved
from 2.98–3.00 s to 2.91–2.96 s. The smaller capture stayed within run-to-run noise at about
2.22–2.30 s, so no broader claim is made. Native tests passed (205 processing, 44 platform).

### Rejected optimizations

Each of these preserved exact output fingerprints and was reverted. They are recorded so they are
not retried without a materially different design.

| Approach | Result | Why it failed |
| --- | --- | --- |
| Fused encoder eligibility checks | No wall-time improvement | Not on the critical path |
| Optimistic constant-cadence encoder path | No wall-time improvement | Not on the critical path |
| Hoisting packed block access out of the per-trigger scan | −16% process CPU, but `scan.dsl` wall 2.23 s → 2.51–2.61 s | Average parallel utilization fell from ~6.5 to 5 cores — CPU saved off the critical path, parallelism lost on it |
| Natural owned-batch adoption at the collector/derived-store handoff | 2.23–2.24 s and 2.95–2.99 s: indistinguishable | ~64.9K-word decoder batches and 131,072-word derived blocks fall out of phase after the first append, so only the first allocation copy is removed |
| Forcing large owned batches to begin new blocks | Collector CPU 1.26 s → 1.06 s, but block count 1,063 → 2,205, storage 188.8 MB → 200.7 MB, `scan.dsl` 2.26–2.33 s | Made the transfer effective by degrading block geometry and storage |
| Segmented codec input, reference table per block | Collector CPU 1.26 s → 1.03 s, but total CPU ~14.6 s → 15.1 s, larger-capture wall unstable to 3.25 s | Table construction cost exceeded the copy it removed |
| Segmented codec input, direct segmented iterators | Encoder CPU rose to 5.1 s / 6.4–6.5 s; wall 2.23–2.34 s and 2.98–3.06 s | Iterator overhead replaced the copy at parity |
| Shared `Arc<Vec<T>>` decoded batches across fan-out | `scan.dsl` 2.25–2.27 s → 2.98–3.75 s; larger 2.96–3.00 s → 3.79–3.92 s; total CPU up | Eliminating one shallow fan-out clone did not offset shared-ownership and downstream materialization costs |
| `miniz_oxide` / `zlib-ng` inflate backends | Slower or no headroom on both captures | See the backend table above |

Sampled call stacks on the larger capture, for orientation on where CPU actually sits:

| Top-of-stack attribution | Samples |
| --- | --- |
| Ordered block extension | 954 |
| Block encoding | 791 |
| Packed block access | 776 |
| Fragment scanning | 676 |
| Presence summaries | 557 |
| Merge closure | 516 |

The coordinator merge closure — the intuitive suspect — is the smallest of these.

## Lessons learned

1. **Artifact count, not data volume, dominated both storage pipelines.** Two independent
   subsystems reached the same conclusion: publication cost scaled with the number of files
   published. Segmentation was worth roughly an order of magnitude in publication latency in both.
2. **Cumulative CPU across overlapping workers is not a critical-path measurement.** The rejected
   packed-block-access hoist reduced process CPU by 16% and made the application slower. Always
   report both, and treat wall time as the acceptance axis.
3. **Measure the policy the application runs.** The probe harness and the production runtime
   differed by a factor of nine in wall time on the same workload. A benchmark that omits
   scheduling policy measures something else.
4. **Ownership-transfer wrappers did not pay.** Four separate attempts — owned-batch adoption,
   forced block starts, two segmented-codec-input designs, and shared `Arc` batches — all preserved
   fingerprints and all failed to improve end-to-end time. The common cause is that the codec
   requires each encoded block to be one contiguous `Vec<Word>`, so a copy is paid somewhere
   regardless of who owns the allocation. Further work here needs a producer/storage representation
   changed end to end, not another wrapper.
5. **Batch geometry is a system property.** Producer batch size, block size, and staging size
   interact; tuning one in isolation moved cost rather than removing it. The retained bounds were
   each validated against a neighbor value.
6. **Local micro-optimization is exhausted on the current representation.** Every isolated loop
   probe after output coalescing was rejected. The remaining candidates are coordinated changes
   spanning producer and consumer, which is why the optimization backlog is ordered around
   avoiding repeated work first and representation changes second.
7. **Throughput and responsiveness are measured together or not at all.** The viewer runs against
   the durable workload in every acceptance run, which is what keeps the 8 ms budget honest.

## Where to look next

The prioritized backlog is in [`TODO.md`](../../TODO.md). Its order reflects the evidence above:

1. **Avoid repeated work across cache and graph generations** — highest expected payoff, because it
   can remove whole reads, decompressions, decodes, or encodes rather than making an already
   parallel kernel marginally faster. Begins with archive work attribution, which must prove
   duplicate work before ownership or scheduling changes.
2. **Coordinate production, storage, and scheduling end to end** — begins with per-batch
   correlation IDs and a reconstructed critical-path trace that separates runnable time from
   queue/backpressure wait, so optimization targets the wall critical path rather than the largest
   cumulative CPU counter.
3. **Interactive responsiveness and perceived latency** — current viewer measurements are already
   inside the frame budget, so this prioritizes avoiding redundant and stale work over raw
   throughput.
4. **GPU only where data is regular and reuse amortizes transfer** — remains conditional, and
   currently unjustified.

Supporting work: a reproducible opt-in regression harness with warmup policy, alternating A/B
order, median and spread, exact identity checks, and retained baseline metadata, so noisy or
microbenchmark-only improvements are hard to accept; telemetry-overhead measurement, so
observability cannot become the bottleneck it exists to diagnose; and equivalent browser-worker
baselines, since native improvements are not assumed to transfer to wasm without measurement.
