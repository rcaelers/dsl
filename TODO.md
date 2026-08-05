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
  protocol and acquisition state machine in `logic_analyzer_processing` and execute that identical implementation on
  a native background executor or browser worker. Model cancellation without pretending that WebUSB can abort one
  transfer independently; closing a web device may be required to abort its outstanding operations.
- [capture.web.usb-access-preflight] Add a generic asynchronous capture-source access preflight started directly by a
  user gesture. It lets the web host call `requestDevice()` without teaching the UI about USB or U3Pro16, and reports
  unsupported browsers, insecure contexts, denied permission, and unavailable devices as source capabilities and
  user-facing diagnostics.
- [capture.web.usb-worker-session] Establish a worker-owned browser USB session after window permission is granted.
  Resolve the permitted U3Pro16 by VID/PID, validate its runtime identity, select configuration 1, claim interface 0,
  handle reconnect/disconnect, and conservatively select High-Speed acquisition limits unless the effective link
  speed can be established from hardware-validated descriptors.
- [capture.web.usb-fpga-image] Define and implement a lawful browser FPGA-image acquisition policy. The application
  website does not bundle or redistribute `DSLogicU3Pro16.bin`, and users must not have to install DSView merely to
  obtain it. Already-configured devices proceed without an upload. An unconfigured or incompatible device requires
  an independently downloadable vendor-authorized image or an image explicitly selected by the user; if neither is
  available, report that capture cannot configure the FPGA. Persist a user-supplied image only with explicit consent.
- [capture.web.usb-adapter] Implement the WebUSB U3Pro16 transport and source-factory override in
  `logic_analyzer_platform`. Translate WebUSB promises, endpoint numbers, control-request fields, transfer statuses,
  short transfers, stalls, timeouts, cancellation, and disconnects into the portable transport contract. Preserve the
  existing protocol and capture behavior; never substitute a synthetic live source.
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

### Capture indexing and caching

- [capture.index.acceleration] Improve finite waveform-index and cache-generation throughput in this
  order:
  1. [x] Profile representative captures, reporting separate read/decompression, packed-block
     handoff/copy, summary-kernel, and artifact-publication timings. The initial `scan.dsl` cold
     build attributes 1.61 s of its 1.66 s wall time to source reading/decompression; summary work
     consumes 0.38 cumulative CPU-seconds, while copying and in-memory artifact publication consume
     about 22 ms and 15 ms respectively. After CPU optimization, a second 2.73 GB packed-input
     profile completes in 1.71 s with 7.66 cumulative worker-seconds in reads and 0.73 in summaries,
     confirming source reading/decompression remains the critical path.
  2. [x] Optimize the CPU path first: remove avoidable packed-block copies and keep bounded source
     read, CPU summary, and artifact-write work pipelined through the existing host executor. Local
     workers retain shared `BlockData` backing, each bounded worker owns one source reader, and the
     coordinator publishes completed leaves in per-channel order. Five post-change `scan.dsl` runs
     have a 0.76 s median, versus 1.66–1.69 s before the change, for an approximately 2.2× median
     speedup with zero handoff-copy time. Isolated native-durable profiles identify the next
     bottleneck: publishing 605 leaf files takes about 3.4 s of a 3.44–4.04 s parallel build, and
     publishing 1,309 leaf files takes about 7.56 s of a 7.59–8.49 s parallel build. Two and four
     workers perform equivalently within about 1% on both captures, while 20 workers are 11–17%
     slower and consume more CPU. Do not impose that native result as a generic executor cap;
     remove the per-leaf publication overhead instead.
  3. [ ] Prototype a batched GPU implementation only for the regular packed digital waveform-summary
     kernel, retaining it only when it beats the optimized CPU baseline while producing bit-exact
     leaf artifacts with the same cancellation, bounded-memory, and progress behavior. The current
     20-worker profiles do not justify starting this prototype: summary work is already off the
     critical path, and GPU dispatch would additionally transfer 1.25–2.73 GB of packed input.
  4. [ ] Preserve platform boundaries: `signal_capture` owns only the portable kernel contract and
     CPU fallback; `logic_analyzer_platform` owns native and WebGPU adapters, capability
     discovery, batching, and unavailable-GPU handling. Do not add target conditionals or GPU
     dependencies to portable processing, viewer, compiler, or concrete-node crates. Keep
     decompression, source I/O, protocol decoding, and derived-data caching on their current CPU
     paths unless measurements identify a separate regular, transfer-efficient kernel.

- [x] [capture.index.segmented-artifacts] Replace one-file-per-waveform-leaf publication with
  bounded immutable segment artifacts and record each leaf's segment offset and length in the root.
  The format groups 64 channel-major leaves per segment, retains a four-segment immutable-region
  cache, publishes the root last, and rejects pre-segment format versions for automatic rebuild.
  `scan.dsl` now publishes 10 segments instead of 605 leaves; durable publication falls from about
  3.4 s to 0.10 s and the best sweep wall time falls from 3.44 s to 0.34 s. The 1,309-leaf capture
  publishes 21 segments; publication falls from about 7.56 s to 0.18 s and best wall time falls from
  7.59 s to 0.60 s. Both post-change sweeps peak at 12 workers and regress at 16–20, so finite index
  builds cap their bounded worker pool at 12 to preserve host capacity and responsiveness.

### Derived-data storage

- [x] [derived.storage.profile] Profile graph-level derived cache generation with a prebuilt
  waveform index and an isolated native durable repository. `scan.dsl` publishes 2,753 immutable
  block files containing 591 MB and spends 2.61 cumulative seconds in block create/write/truncate/
  rename calls during a 3.04 s pipeline; its 14 final index/manifest pairs consume about 0.37
  cumulative seconds including durability barriers. The larger capture publishes 3,237 block files
  containing 741 MB and spends 3.02 cumulative seconds in those calls during a 4.02 s pipeline;
  final index/manifest publication consumes about 0.33 cumulative seconds. Repository call times
  can overlap, but the artifact counts, system CPU, and scaling consistently identify per-block
  filesystem publication as the storage bottleneck and justify segmentation.
- [x] [derived.storage.segmented-artifacts] Replace one-file-per-derived-block publication with a
  bounded number of large immutable segment artifacts. Encode blocks concurrently, append their
  ordered bytes into segment-sized writable mappings or buffered regions, and publish only complete
  segments plus the final index/manifest generation. Native mappings rely on ordinary OS page-cache
  writeback rather than a durability barrier per block; web storage uses the same segment/index
  model over its injected repository. Preserve atomic generation visibility, cancellation cleanup,
  exact range queries, cache portability, and corruption validation. Use `logic-conduit run
  graphs/spi_controlled_decode.json --json` as the end-to-end acceptance benchmark and keep artifact
  count, bytes, execution time, CPU utilization, and final-publication latency visible in its report.
  1. [x] Introduce versioned segment keys and extend each persistent directory record with its
     segment sequence, byte offset, and length; reject the block-per-file index version for rebuild.
  2. [x] Keep concurrent block encoding, restore sequence order at commit, and append encoded bytes
     into a bounded active segment without a durability barrier per block; profile the bound across
     concurrent lanes rather than treating one lane's target as the process-wide memory cost.
  3. [x] Preserve live queries through a bounded in-memory view of blocks in the unpublished active
     segment; publish complete segments atomically and release their staging buffers.
  4. [x] Read exact block ranges from immutable segment regions and retain existing checksum,
     directory, presence-index, missing-artifact, and corruption validation.
  5. [x] Migrate cleanup, cancellation, cache inspection, LRU accounting, native/web repository
     conformance tests, and automatic rebuilding from the prior block namespace.
  6. [x] Re-run `derived-storage-profile` on both reference captures and accept the format only if
     it materially reduces artifact count, wall time, and system CPU without regressing output
     fingerprints, exact queries, or final-publication latency.
     An 8 MiB per-lane target retains the filesystem gain without the 32 MiB prototype's higher
     aggregate staging footprint. Across repeated `scan.dsl` runs it publishes 82 segments instead
     of 2,753 blocks, has a 2.92 s median wall time versus 3.04 s, and has a 2.40 s median system-CPU
     time versus 3.67 s. The larger capture publishes 99 segments instead of 3,237 blocks, completes
     in 3.58–3.72 s versus 4.02 s, and consumes 2.79–3.05 s system CPU versus 5.10 s. Stored bytes,
     word counts, and both output fingerprints are unchanged. The profiler now records each lane's
     actual index-to-manifest publication span separately from overlapping cumulative repository-call
     time; the slowest lane is 78 ms for `scan.dsl` and 44 ms for the larger capture.

### Graph execution

- [x] [runtime.performance.post-segmentation] Re-profile and optimize the runtime after derived and
  waveform artifact segmentation changed the critical path.
  1. [x] Attribute post-segmentation execution to concrete processing-node work, derived-block
     encoding, graph scheduling/backpressure, segment publication, and final metadata publication.
     Report both critical-path wall time and overlapping cumulative CPU/work time.
  2. [x] Exercise the production viewer while the same durable-cache workload runs, reporting lane
     query latency, pointer-input frame p50/p95/p99, frames beyond 8/16 ms, CPU utilization, and peak
     resident memory so throughput changes cannot consume foreground responsiveness.
  3. [x] Optimize the measured CPU stage on the critical path, preferring bounded batching,
     allocation reuse, or scheduling/backpressure improvements; preserve output fingerprints,
     cancellation, exact queries, memory bounds, and native/web behavior.
  4. [x] Reassess the GPU prototype only after the new CPU baseline. Keep it deferred unless a
     regular, batchable, transfer-efficient kernel remains on the critical path and an accelerated
     implementation beats the CPU path without weakening portability or responsiveness.
     Diagnostic-only executor labels now separate DSL block reading, parallel fragment scans, and
     derived-block encoding; sampled node metrics expose work-call count, wall latency, and thread
     CPU without changing execution policy. This found a benchmark/production discrepancy: the old
     no-backoff probe completes `scan.dsl` in 2.86 s but consumes 21.73 CPU-seconds, while the native
     runtime's fixed 2 ms idle delay consumes 16.69 CPU-seconds but stretches the same run to 24.62 s.
     The threaded manager now honors `WorkOutcome::made_progress`, briefly yields through the
     injected executor, and then uses a 50 us idle backoff. It completes `scan.dsl` in 2.70 s using
     15.32 CPU-seconds and the larger capture in 3.56 s using 19.92 CPU-seconds, with unchanged output
     fingerprints. During the durable `scan.dsl` workload, pointer-input frames have 0.50/1.01/1.05
     ms p50/p95/p99 latency, lane queries have 0.41/0.54/0.67 ms latency, and no frame exceeds 8 ms.
     Remaining CPU is distributed across source reading, fragment scanning, ordered decoder work,
     variable-length derived encoding, and sinks rather than one regular transfer-efficient kernel;
     the GPU prototype therefore remains unjustified.

- [runtime.performance.parallel-merge] Optimize the serialized Parallel Decoder path after fragment
  scanning.
  1. [x] Measure input/dispatch, ordered-completion wait, merge/word assembly, sampling-point
     publication, and output-batch send time separately on both reference captures.
  2. [x] Reduce the dominant serialized phase with bounded producer-owned batches, fragment
     coalescing, or allocation reuse while retaining ordered completion and backpressure.
  3. [x] Verify identical output and derived-lane fingerprints, bounded in-flight/reorder memory,
     cancellation latency, durable-cache throughput, and concurrent viewer frame/query latency.
  4. [x] Record whether another CPU optimization remains worthwhile before reconsidering GPU work.
     Opt-in phase counters showed that completion waiting is negligible and that merge plus durable
     sampling publication was the largest serialized section. The persistent sampling store now
     owns an opaque, storage-ready word batch, so the decoder encodes directly into the queued
     writer's representation instead of first retaining a second `Vec<PackedSamplingPoint>` and
     converting it during publication. On `scan.dsl`, sampling publication falls from 709 ms to
     49 ms, merge plus publication from 2.05 s to 1.60 s, and pipeline wall time from 2.70 s to
     2.43 s. On the larger reference capture, those measurements fall from 845 ms to 52 ms,
     2.46 s to 1.91 s, and 3.56 s to 3.05 s respectively. Output fingerprints, derived word counts,
     and stored bytes are unchanged. The existing 65,536-sample fragment bound is retained; the
     `scan.dsl` peak is 468 MB, while doubling the fragment size had previously raised it to 536 MB
     for only a small wall-time gain. Cancellation remains bounded by the fragment window. During
     the durable live-viewer workload, pointer-input frames have 0.50/1.02/1.18 ms p50/p95/p99
     latency, lane queries have 0.39/0.55/0.68 ms latency, and no frame exceeds 8 ms. Remaining time
     is split among merge/assembly, fragment scans, source reads, derived encoding, output sends,
     and sinks; there is still no single transfer-efficient kernel that justifies GPU acceleration.

- [runtime.performance.parallel-output-coalescing] Reduce remaining Parallel Decoder merge/output
  allocation and envelope overhead.
  1. [x] Measure output batch count, words per batch, bounded pending capacity, destination fan-out,
     and the relationship between output-send time and downstream collector calls.
  2. [x] Reuse one decoder-owned merge batch and coalesce adjacent ordered fragments up to a fixed
     word bound, flushing the tail before end-of-stream without weakening channel backpressure.
  3. [x] Compare both reference captures against the 2.43 s and 3.05 s durable baselines; retain the
     change only with identical output/derived fingerprints and a justified peak-memory tradeoff.
  4. [x] Re-run cancellation, concurrent viewer latency, native/wasm tests, and lint, then record the
     next evidence-backed optimization or stop point.
     The decoder now merges directly into one ordered pending batch bounded at 65,536 words, the
     maximum output of one existing fragment, and flushes a partial tail through the shared streamed
     lifecycle before end-of-stream. On `scan.dsl`, 13,830 fragment scans become 2,179 output
     batches, output-send time falls from 327 ms to 230–242 ms, retained-collector calls fall from
     129,834 to 41,154, and file-writer calls fall from 8,425 to 2,108. Repeated durable runs complete
     in 2.25–2.27 s after one 2.45 s cold outlier, versus the 2.43 s prior baseline. On the larger
     capture, 30,286 fragment scans become 2,728 batches, output-send time falls from 378 ms to 260 ms,
     retained-collector calls fall from 158,439 to 57,462, and file-writer calls fall from 10,634 to
     2,635. Repeated runs complete in 2.96–3.00 s versus 3.05 s. Fingerprints, derived word counts,
     and stored bytes remain identical. The largest observed pending batch is 64,900 words; peak RSS
     is 436–458 MB on `scan.dsl` and 456–478 MB on the larger capture, below the rejected 536 MB
     double-fragment experiment. A 32,768-word probe increased send counts without improving wall
     time or observed peak memory. The concurrent viewer reports 1.41 ms p99 pointer-input frames,
     0.91 ms p99 queries, and no frame over 8 ms. Further output coalescing is not justified; the
     next speed investigation should sample the merge loop and derived encoder at function level
     rather than increase batch or fragment bounds.

- [runtime.performance.merge-encoder-functions] Profile and optimize the remaining merge-loop and
  derived-word encoder CPU at function level.
  1. [x] Capture sampled call stacks and per-function attribution for both reference workloads,
     separating coordinator merge work from queued derived encoding.
  2. [x] Optimize only the hottest bounded operation while preserving generic storage contracts,
     concrete-node ownership, ordered output, cancellation, and portable native/wasm source.
  3. [x] Re-run durable wall/CPU/memory and exact fingerprint comparisons on both captures; revert
     any change that merely shifts time between overlapping workers or regresses responsiveness.
  4. [x] Validate the concurrent viewer, cancellation, native/wasm tests, and lint, then record the
     next evidence-backed target or stop point.
     Sampled stacks on both captures place packed fragment scanning and generic derived-word block
     construction ahead of the coordinator merge closure. On the larger capture, representative
     top-of-stack samples attribute 954 samples to ordered block extension, 791 to block encoding,
     776 to packed block access, 676 to fragment scanning, 557 to presence summaries, and 516 to
     the merge closure. Three bounded probes were rejected: fusing encoder eligibility checks and
     an optimistic constant-cadence encoder path did not improve wall time, while hoisting packed
     block access out of the per-trigger scan reduced process CPU by about 16% but increased the
     `scan.dsl` wall time from 2.23 s to 2.51–2.61 s as average parallel utilization fell from about
     6.5 to 5 cores. All probes retained exact output fingerprints and were reverted. The accepted
     coalesced-output baseline remains the faster interactive result and retains its previously
     validated cancellation, viewer-latency, native/wasm, and lint results. There is no justified
     local merge/encoder micro-optimization to retain. A further investigation would need to treat
     ownership transfer into generic derived-store builders and worker scheduling as a coordinated
     critical-path change, rather than optimize another isolated loop.

- [runtime.performance.derived-builder-ownership] Remove avoidable decoded-word copying at the
  collector-to-derived-store handoff without increasing scheduling latency.
  1. [x] Measure batch ownership, builder occupancy, allocation reuse, encoder dispatch, and the
     overlap between collection, encoding, persistence, and the decoder's critical path.
  2. [x] Add a generic owned-batch writer contract and let an empty block builder adopt a complete
     ordered input allocation when it fits, retaining the borrowed path for shared callers.
  3. [x] Compare both reference captures with the coalesced-output baseline, including exact
     fingerprints, elapsed/CPU time, peak memory, adopted-batch rate, and encoder utilization.
  4. [x] Retain the change only if end-to-end or interactive latency improves; validate bounded
     cancellation, concurrent viewer queries, native/wasm tests, and lint, then record a stop point.
     The collector owns its channel batches, but the approximately 64.9K-word decoder batches and
     131,072-word derived blocks remain out of phase after the first append. A natural owned-batch
     adoption therefore removes only the first allocation copy: warmed `scan.dsl` runs complete in
     2.23–2.24 s and the larger capture in 2.95–2.99 s, indistinguishable from the retained
     2.25–2.27 s and 2.96–3.00 s ranges. Forcing large owned batches to begin new blocks makes the
     transfer effective and lowers retained-collector CPU from about 1.26 s to 1.06 s, but doubles
     the large lane's block count from 1,063 to 2,205, increases total durable derived storage from
     188.8 MB to 200.7 MB, and slows `scan.dsl` to 2.26–2.33 s. Output fingerprints and word counts
     remain exact in both probes. Both implementations were reverted; the accepted baseline keeps
     its existing bounded cancellation and concurrent-viewer results, and the final native/wasm
     tests and lint pass. Ownership transfer is not useful while the codec requires each encoded
     block to be one contiguous `Vec<Word>`. Revisit only as part of a segmented-input codec design
     that can encode owned chunks directly without changing block boundaries or flattening first.

- [runtime.performance.segmented-codec-input] Let derived-word encoding retain owned input chunks
  while preserving codec block boundaries and durable format identity.
  1. [x] Replace contiguous builder storage assumptions with a generic segmented word view used by
     block sizing, presence summaries, hot-tail publication, and encoding.
  2. [x] Transfer collector-owned batches into the builder, splitting only boundary fragments and
     preserving the borrowed append contract for shared and incremental callers.
  3. [x] Compare both reference captures against the retained baseline with identical output and
     derived fingerprints, block counts, stored bytes, elapsed/CPU time, and peak memory.
  4. [x] Retain only an end-to-end or interactive improvement; validate cancellation, concurrent
     viewer queries, native/wasm tests, lint, architecture boundaries, and document the stop point.
     Two exact-format segmented prototypes retained collector-owned `Vec<Word>` allocations behind
     shared ranges, including batches split across asynchronous block encoders. Both preserve the
     `scan.dsl` fingerprint, 136,939,197 derived words, 1,063 Parallel Decoder blocks, and 188.8 MB
     durable footprint; the larger capture likewise preserves its fingerprint, 171,356,637 words,
     1,321 blocks, and 235.8 MB footprint. Materializing one reference table per block lowers
     retained-collector CPU from about 1.26 s to 1.03 s, but raises total CPU from about 14.6 s to
     15.1 s and produces unstable larger-capture wall times up to 3.25 s. Encoding directly through
     segmented iterators avoids that table but raises encoder CPU to 5.1 s on `scan.dsl` and
     6.4–6.5 s on the larger capture; wall time remains 2.23–2.34 s and 2.98–3.06 s, no reliable
     improvement over the retained 2.25–2.27 s and 2.96–3.00 s baselines. Both implementations
     were reverted. The final baseline retains its already validated cancellation and concurrent
     viewer behavior, and native/wasm tests and lint pass. Further decoded-word ownership work is
     not justified without changing the producer/storage representation end to end so encoding
     does not pay either a full-word copy, a reference-table pass, or segmented-iterator overhead.

- [runtime.performance.shared-decoded-batches] Evaluate an end-to-end shared decoded-batch
  representation across producer fan-out and storage consumers.
  1. [x] Attribute allocation and cloning across decoder merge output, generic port fan-out,
     retained collection, derived encoding, and file-writer consumption on both references.
  2. [x] Introduce the smallest protocol-neutral immutable batch contract that lets independent
     consumers share decoded values while preserving typed payload APIs and backpressure.
  3. [x] Compare exact output and derived identities, wall/CPU time, peak memory, fan-out copies,
     cancellation, and concurrent viewer latency against the retained baseline.
  4. [x] Retain only a measurable end-to-end or interactive improvement; validate native/wasm
     tests, lint, architecture boundaries, and record the next target or stop point.

  The measured fan-out copy is the decoder's `Vec<Word>` clone for each additional destination;
  the retained collector then traverses those values again while building its contiguous encoded
  block. A protocol-neutral `Arc<Vec<T>>` message prototype shared the decoder allocation across
  the file writer and retained collector, with legacy receivers retaining their existing owned
  batch behavior. Exact file-output fingerprints, derived word counts, block counts, and durable
  byte totals stayed unchanged on both references. End-to-end performance regressed, however:
  `scan.dsl` rose from the retained 2.25–2.27 s range to about 2.98–3.75 s, while the larger capture
  rose from 2.96–3.00 s to 3.79–3.92 s; total CPU also increased. The prototype was reverted. The
  result closes shared transport envelopes as a useful next step: eliminating one shallow fan-out
  clone does not offset shared-ownership and downstream materialization costs, so further work
  needs a materially different producer/encoding algorithm backed by a new profile, not another
  ownership wrapper.

- [runtime.performance.native-positional-reads] Reduce native capture and index source I/O overhead
  without changing the portable random-access contract.
  1. [x] Sample the retained workload again and separate native file reads/seeks, ZIP inflation,
     packed summary construction, and downstream runtime work.
  2. [x] Replace cursor-mutating native reads with positional reads in the platform adapter and the
     explicitly allowlisted processing file adapter, retaining independent-reader and bounds/error
     semantics with a non-Unix fallback where required.
  3. [x] Compare isolated waveform-index generation and both durable runtime references against the
     retained baseline, including exact artifact/output identities, CPU, wall time, and peak memory.
  4. [x] Retain only a repeatable improvement; validate native/wasm tests, lint, architecture
     boundaries, cancellation, and concurrent viewer responsiveness, or revert and record why.

  The refreshed native sample places source reads and ZIP inflation ahead of packed-summary work;
  seek calls are visible but not themselves dominant. Unix readers in both the platform adapter and
  the explicitly allowlisted processing adapter now use positional file reads, while non-Unix hosts
  retain the cursor fallback. Alternating exact-build A/B runs show a small but repeatable effect on
  the larger reference: warm waveform-index read work falls by about 2–4%, and durable runtime wall
  time moves from 2.98–3.00 s to 2.91–2.96 s. The smaller capture remains within run-to-run noise at
  about 2.22–2.30 s after the cold run, so no broader claim is warranted. Both captures preserve
  their exact output fingerprints, derived word counts, block counts, and durable byte totals; the
  index profiles preserve 605/1,309 blocks and 1.246/2.729 GB of packed input. Native tests pass
  (205 processing and 44 platform), wasm checks and lint pass, and the concurrent viewer reports
  1.22 ms p99 input frames, 0.69 ms p99 queries, and no frame above 8 ms. The improvement comes from
  a host I/O primitive, not a GPU-suitable kernel; ZIP inflation remains the next measurable index
  cost, but requires a separate algorithm/backend comparison before changing dependencies or format.

- [capture.index.inflate-backend] Compare portable ZIP/DEFLATE implementations on cold waveform
  index generation before changing the capture archive stack.
  1. [x] Identify the active decoder and feature unification: `zip` currently selects `flate2` with
     the pure-Rust `zlib-rs` backend; unrelated image dependencies also enable `miniz_oxide`.
  2. [x] Build an otherwise identical `miniz_oxide` candidate and alternate cold and warm index
     profiles for both captures, preserving ZIP compatibility and exact index dimensions.
  3. [x] Check whether any faster native-only backend has enough measured headroom to justify an
     injected platform capability; do not introduce target selection into processing or core code.
  4. [x] Retain only a portable, repeatable end-to-end improvement; otherwise restore `zlib-rs`,
     validate the retained native/wasm build, and record the next target or stop point.

  Alternating exact-build profiles reject both alternatives. On `scan.dsl`, warmed `zlib-rs`
  builds complete in 0.30–0.34 s with 1.84–1.91 cumulative read/decompression CPU-seconds;
  `miniz_oxide` needs 0.40–0.41 s and 2.82–2.89 CPU-seconds. On the larger capture, warmed
  `zlib-rs` completes in 0.53–0.56 s with 2.60–2.82 read CPU-seconds, versus 0.70 s and 4.19
  CPU-seconds for `miniz_oxide`. A native `zlib-ng` upper-bound probe also provides no useful
  headroom: small-capture warm wall and CPU are slightly worse, while larger-capture wall overlaps
  and read CPU rises from 2.60 to about 2.80 s. Every backend preserves 605/1,309 blocks and
  1.246/2.729 GB of packed input. Both prototypes and their dependency changes were reverted;
  `zlib-rs` remains the portable backend. No injected native decompression capability is justified.
  Further index acceleration should target archive-level work scheduling or reuse, and must begin
  with evidence of duplicate decompression on a real critical path rather than another codec swap.

### Optimization backlog (future, priority order)

The completed investigations above define the retained baseline and the rejected approaches. Apply
the same acceptance rule to every item below: compare both reference captures, exact output and
artifact identities, wall and CPU time, peak memory, cancellation bounds, native/wasm behavior, and
concurrent viewer p99 latency. Do not retain a throughput change that harms foreground response.

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
     in `logic_analyzer_processing`.
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
   - [ ] [platform.gpu-capability] If any GPU prototype wins, define the capability and portable CPU
     fallback in the owning core crate, implement native/WebGPU adapters only in
     `logic_analyzer_platform`, inject them at composition roots, and expose availability/fallback
     diagnostics. Never make cache identity depend on the selected device.

- [performance.regression-harness] Turn the existing capture benchmarks into an opt-in reproducible
  comparison report with warmup policy, alternating A/B order, median and spread, exact identity
  checks, peak RSS, CPU, viewer percentiles, and retained baseline metadata. Keep large captures out
  of ordinary unit tests, but make it difficult to accept noisy or microbenchmark-only improvements.
- [performance.telemetry-overhead] Measure profiling counters disabled and enabled; sample or aggregate
  hot-path metrics so observability cannot become the bottleneck it is intended to diagnose.
- [performance.web-baselines] Establish equivalent browser-worker baselines for waveform generation,
  derived caching, graph edits, and viewer input latency using the same artifact identities and
  bounded-memory rules. Native improvements are not assumed to help wasm without measurements.

- [graph.execution.debounced-live-sync] Replace fixed-interval semantic graph polling with an
  event-driven dirty revision and a true debounce: reset the quiet-period timer after every
  processing-relevant edit, lower only the latest immutable graph revision after the quiet period,
  and discard stale results when a newer revision exists. Perform lowering and edit-plan
  preparation away from the UI thread, keep runtime application ordered through its control
  boundary, and leave periodic progress reporting independent from graph synchronization.

### Capture provider and host architecture

- [capture.live.provider-unification] Represent file and live sources through one generic capture
  data-provider contract for presentation, readiness, cache/index availability, and data access.
  Providers advertise optional acquisition commands and capabilities, so file sources do not
  pretend to support live acquisition and the application does not branch on file-versus-live
  source kinds to publish artifacts or attach viewer data.
- [capture.live.host-capabilities] Add a host capability that inhibits automatic system sleep while
  acquisition is active. Where inhibition is unavailable, observe suspend/resume and report it as
  a capture-integrity event. Keep the existing generic lifecycle, integrity, and storage contracts
  in `signal_capture_session`, with no platform conditionals in their consumers.

### Node-graph extraction

- [graph.extraction.standalone-crate] Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
