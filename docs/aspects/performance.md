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

The portable `routing_performance_tests` fixture measures three CPU-side costs separately:
`rebuild_routes` on a prepared layout, an average of 32 point-to-wire hover queries, and
`Context::run_ui` plus editor rendering and egui tessellation. The CPU frame includes the
editor's layout/routing passes, but excludes texture uploads, GPU submission, presentation,
and the surrounding application. It is not an end-to-end display-frame measurement.

`paired-grid-v1` has 100 nodes/500 connections or 500 nodes/2000 connections. Each neutral
node has ten inputs and ten outputs. Disjoint source/target pairs occupy five columns,
900 units apart horizontally and 700 vertically; each target is 450 units right of its
source. The smaller fixture connects all ten matching ports per pair, the larger eight.
A fixture assertion keeps every body and its 60-unit escape envelope disjoint. Zoom is
0.35 and the logical egui viewport is 1440 × 900, so routing includes offscreen geometry.

Routing snapshots are rebuilt without history. Layout preparation, including its initial
route build, is outside the isolated routing timer. The first measured sample is recorded
separately; subsequent samples use warmed allocator/font/egui state, not a cold process.
Release runs collect twenty subsequent samples and report nearest-rank p50/p95, maximum,
and sorted raw samples. Debug tests run a cold/repeated two-sample correctness smoke check.
Every sample asserts complete finite path presentation, stable routing outcome counts, and
unchanged topology sizes; there are no hardware-dependent timing assertions. Fallback
counts and reasons accompany timings so work exhaustion cannot masquerade as throughput.

Reproduce the native measurement with:

```sh
cargo test -p node-graph --release routing_scale_native -- --nocapture
```

The browser uses the same fixture and measurement body:

```sh
NO_HEADLESS=1 CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test -p node-graph --release --target wasm32-unknown-unknown --lib \
  routing_scale_browser -- --nocapture
```

Open the runner's localhost URL and retain the `ROUTING_PERFORMANCE` console JSON.
`ROUTING_PROGRESS` messages identify each construction, routing, and frame sample outside
the measured intervals. The runner must match the lockfile's wasm-bindgen version (0.2.127
for this baseline). Browser execution is interactive, with no GPU-backed canvas in this
harness; a long synchronous test can keep the page unresponsive until it finishes.

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

Interactive Chrome execution reached both corrected fixtures, but automation stalled
before a completed test result and full JSON report could be retrieved. Partial progress
logs are not retained as a browser baseline or a browser correctness pass. Repeating the
browser measurement with a reliably bounded runner remains open.

### Proposed future measurements

Real application/GPU frame timing, moving-node workloads, post-drag quality passes, and
history-aware comparison remain required by the connection-routing plan. The CPU-only
scale harness does not replace those acceptance gates.

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
