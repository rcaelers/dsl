//! # `signal_processing` Runtime Design
//!
//! Design of the generic `signal-processing` crate
//! ([crates/signal_processing](../../../crates/signal_processing)): the UI-free streaming engine that
//! executes pipelines. Concrete logic-analyzer nodes live in
//! [`logic_analyzer_processing`](../../../crates/logic_analyzer_processing); the node-graph editor and
//! graph→pipeline compiler live above both (see [Application Composition Design](../../architecture/application_composition.md)).
//!
//! ---
//!
//! ## Architecture
//!
//! Thread-per-node streaming with bounded channels:
//!
//! - Every node implements `ProcessNode` and runs on its own OS thread (true parallelism;
//!   no locking in node logic).
//! - Nodes are connected by bounded crossbeam channels created from the graph description;
//!   one output can broadcast to any number of inputs.
//! - **Backpressure is automatic and genuine**: a full channel blocks the producer, and the
//!   stall propagates upstream. This is deliberate (see *Flow control philosophy* below).
//! - Shutdown is a cascade: when a source finishes (or is dropped), closed channels surface as
//!   `WorkError::Shutdown` in downstream `recv()`/`send()` calls, unwinding each node cleanly.
//!
//! ```text
//! ┌────────────────┐      ┌──────────────┐      ┌────────────┐
//! │ DslFileSource  │─────▶│  SpiDecoder  │─────▶│ FileWriter │
//! │   (thread 1)   │      │  (thread 2)  │      │ (thread 3) │
//! └────────────────┘      └──────────────┘      └────────────┘
//!      Sample edges          Word events           files
//! ```
//!
//! ### `ProcessNode`
//!
//! The single node trait ([node.rs](../../../crates/signal_processing/src/node.rs)). Sources have
//! 0 inputs, sinks 0 outputs, processors both.
//!
//! Runtime builders distinguish a time-domain source from an auxiliary zero-input source. A graph
//! has at most one source that establishes its capture/data time domain, while persisted timeline
//! markers and similar auxiliary sources may emit values already expressed in that shared domain.
//!
//! - `work(&mut self, inputs: &[InputPort], outputs: &[OutputPort]) -> WorkResult<usize>` —
//!   process one batch; the returned count is the number of produced items shown in runtime
//!   progress counters. Blocking `recv`/`send` inside `work()` is normal;
//!   `Err(WorkError::Shutdown)` ends the node.
//! - `work_outcome(...) -> WorkResult<WorkOutcome>` — scheduler-facing progress. The default
//!   derives progress from the produced-item count. Filters, framers, matchers, and other nodes
//!   that can consume input or advance state without emitting an item return
//!   `WorkOutcome::progressed(0)` for those calls.
//! - `input_schema()` / `output_schema()` — named, typed ports (`PortSchema` carries the
//!   `TypeId`); this is what makes graph wiring name-based and type-checked.
//! - `is_self_threading()` — a node that manages its own worker threads (e.g. `DslFileSource`
//!   spawns per-channel readers); the scheduler calls `work()` once to start it and then waits
//!   on `should_stop()`.
//! - `apply_config(&NodeConfig) -> ConfigOutcome` — optional hot reconfiguration
//!   (`Applied` / `NeedsRestart`), used by live editing (below).
//!
//! ### `Pipeline` (offline builder)
//!
//! [pipeline.rs](../../../crates/signal_processing/src/pipeline.rs) /
//! [ports.rs](../../../crates/signal_processing/src/ports.rs): name-based graph construction —
//! `add_process("name", node)`, `connect("from", "port", "to", "port")` (or
//! `connect_with_buffer(…, size)`), then `build()` validates names and port `TypeId`s, creates
//! one bounded channel per edge via the **type registry** (`register_type::<T>()` — every
//! payload type that flows through a channel must be registered so the builder can create
//! channels type-erased), fans outputs out to their subscriber lists, injects watchdog context,
//! and hands everything to the `Scheduler`. Endpoints move into node threads; teardown is the
//! drop-cascade. `scheduler.wait()` joins all threads.
//!
//! ## Stream types and the level contract
//!
//! All timestamps are **nanoseconds in one shared domain** — every value ultimately derives
//! its time from the same source position, which is what keeps decoded events, control levels,
//! and viewer lanes aligned (including across live-edit rejoins).
//!
//! | Type | Kind | Meaning |
//! |---|---|---|
//! | `Sample` | level | Logic-level change (RLE edge): `value`, `start_time_ns` |
//! | `SampleBlock` | bulk | Packed raw bits of one channel block (bandwidth path) |
//! | `Word` | event | One decoded item with a numeric value, arbitrary-width bytes, or text plus `timestamp_ns` and `duration_ns` — the standard decoded-value type every decoder emits |
//! | `Trigger` | event | Instantaneous occurrence (matcher hit), time only |
//! | `TimelineMarker` | configuration event | One persisted graph point or host-supplied cursor position transported to marker conversion nodes; names, identities, and external references remain in graph/host contracts |
//! | `NumberSample` | level | Integer level change (counter output) |
//! | `TextSample` | level | Text level change (formatter output / filename) |
//!
//! **The level-stream contract** ([events.rs](../../../crates/signal_processing/src/events.rs)):
//! every low-rate stream is a *stepped level* — defined at every instant, transmitted as
//! changes only. Every level producer emits its initial value at t=0 on its first `work()`
//! call, and consumers hold the last received value. Consequently any node can always answer
//! "what is your value now" (a gate can always combine, the writer always knows the current
//! filename) and **no node ever blocks waiting for a control value to exist** — only data can
//! lag. Only `Word` and `Trigger` are true events, undefined between occurrences: they can
//! only be reacted to.
//!
//! This contract is the system's deadlock guard, and it extends to live editing: level
//! channels are *sticky* — a subscriber added mid-stream is immediately primed with the
//! current value.
//!
//! `Sample` vs `SampleBlock` is a bandwidth concern, not a semantic one: a source exposes each
//! channel both as RLE edges (`d{i}`) and packed blocks (`b{i}`); consumers declare which they
//! take and the compiler picks per edge.
//!
//! ## Random-access signal capabilities
//!
//! `EdgeQuery` is a query capability carried alongside the stream contract. A connection may
//! negotiate it as its transport, or retain it as an auxiliary capability while continuing to
//! stream. Nodes receive their connected input capabilities when their outputs are materialized, so
//! pass-through and combining nodes can expose random-access output queries without generic runtime
//! code knowing their boolean or protocol semantics.
//!
//! Finite raw sources answer through their waveform indexes. Sparse derived level nodes retain only
//! their transitions, while combiners such as logic gates evaluate their output lazily from input
//! queries and convert between input sample rates in the shared time domain. Growing derived queries
//! prefer stream transport during execution so downstream state cannot run ahead of their producer;
//! the auxiliary query is complete through the downstream processing watermark and supports passive
//! post-run inspection. Sampling overlays use these capabilities to reconstruct visible markers
//! without rerunning the pipeline or retaining one marker per decoded sample.
//!
//! ## High-throughput transport and parallel-bus decoding
//!
//! The high-bandwidth path is designed around bounded, shared storage rather than scalar
//! messages or capture-sized queues:
//!
//! - `BlockData` adopts owned `Vec<u8>` allocations, shares `Arc<[u8]>`, and can reference
//!   native mmap storage. Byte-aligned `SampleBlock` subviews retain the same backing.
//! - File sources use a bounded two-window block cache and send aligned channel groups in
//!   lockstep. Default packed-block connections are deliberately small.
//! - `ChannelMessage<T>::Batch(Vec<T>)` amortizes channel overhead. Ordinary receivers flatten
//!   batches transparently; batch-aware sinks preserve the producer-created vectors and can process
//!   several envelopes as scatter/gather slices without copying them into another contiguous batch.
//! - `DerivedDataRetention::Unlimited` preserves finite captures, while continuous sources may
//!   explicitly request bounded rolling retention.
//!
//! `ParallelDecoder` supports `Auto`, `PackedStream`, and `Indexed` input strategies. Indexed
//! mode uses hierarchical transition queries and batched point reads, making it appropriate
//! for sparse signals. Packed mode scans resident 64-bit words and is appropriate for dense or
//! live signals. Auto estimates indexed work from the strobe activity and any cheap, complete CS or
//! Enable high-level occupancy hints. A connected gate without a complete hint remains unknown and
//! does not bias the choice toward the single-threaded indexed path. Packed scanning is selected at
//! an estimated gated activity of 12.5% or greater, calibrated by the complete controlled-decode
//! reference workload. Either choice applies coherently to the raw strobe, data, and CS group; Enable
//! selects its transport independently, and explicit strategies always override Auto.
//!
//! Packed decoding separates immutable scanning from ordered state updates. Each bounded
//! 65,536-sample fragment records trigger positions, bus values, reset markers, and boundary
//! state. Ordered merge repairs fragment-edge transitions, carries partial words, and emits one
//! ordered word batch. The graph build context injects a bounded `WorkExecutor`; platform composition
//! selects a bounded native worker queue or the portable inline executor. A decoder's adaptive default
//! uses one worker on one- or two-way hosts and approximately one quarter of the advertised capacity
//! otherwise, with a minimum of two. This reserves capacity for capture, ordered merge, downstream
//! consumers, and concurrent decoder nodes after packed scanning reaches memory-throughput saturation.
//! Explicit benchmark and tuning overrides accept one through 32 workers and remain capped by the
//! advertised host capacity. Each decoder allows at most `2 * workers` outstanding fragments and
//! reorders completion by sequence. A host that advertises one worker executes the same decoder path
//! sequentially.
//!
//! These boundaries preserve deterministic values/timestamps, bounded memory and backpressure,
//! and responsive cancellation. The opt-in `parallel-decoder-bench` binary reports protocol
//! selection, fingerprints, CPU and packed-input throughput, peak RSS, separate in-flight and reorder
//! fragment estimates, and retention behavior:
//!
//! ```bash
//! cargo run -p logic-analyzer-processing --release --bin parallel-decoder-bench -- --help
//! ```
//!
//! Its `--worker-sweep` mode requests 1, 2, 4, 8, 16, and 32 workers. The `count` sink verifies that
//! every run has the same word count and fingerprint; the `discard` sink removes downstream word
//! transport to characterize the immutable scan stage. Requested counts above the host capacity are
//! reported with their lower effective count. The current adaptive policy follows the scan-only
//! saturation range while avoiding the linear fragment-memory cost of assigning every host worker to
//! one decoder.
//!
//! The benchmark's `file` sink writes decoded words through the production binary-file sink. It
//! therefore covers decoder transport, batch serialization, and filesystem output without retaining
//! the decoded stream in memory.
//!
//! The indexed derived-word consumer retains producer-owned batches until a bounded drain boundary.
//! Its writer prepares complete blocks through the `WorkExecutor` selected in the lane configuration,
//! accepts their completion in any order, and gives one file owner sole responsibility for contiguous
//! ordered publication. One writer uses at most four block tasks and scales its bound from the
//! executor's advertised capacity, reserving remaining capacity for decoder scans, other nodes, and
//! concurrent lanes. This removes block encoding and presence-summary construction from the
//! serialized output path without relaxing deterministic ordering, complete final publication,
//! storage-failure isolation, or backpressure. Collector drains harvest the contiguous prepared prefix
//! without waiting for every dispatched encoder, allowing production and cache encoding to overlap.
//!
//! The native application's headless command is the end-to-end throughput benchmark. It restores the
//! saved graph through the same application policy as the UI and uses the native durable artifact
//! repository, source factories, connected-output retention, subscribed viewer lanes, processing
//! runtime, and file sinks:
//!
//! ```bash
//! cargo run --release --bin logic-conduit -- \
//!   run graphs/spi_controlled_decode.json --json > ui-equivalent-run.json
//! ```
//!
//! The report includes phase timing, capture throughput, final node progress, derived-lane counts, and
//! durable cache sizes. Runtime node failures make the command fail. The command clears that graph's
//! derived cache entries before execution, matching the UI Run command, while retaining the prepared
//! raw capture index.
//!
//! The manual compiler-capture tool loads the checked-in controlled parallel-decoder graph and
//! isolates processing, semantic validation, protocol selection, cancellation, and viewer-query
//! latency from durable repository performance. Its `baseline` command emits graph/capture/output
//! fingerprints, phase timing, average CPU utilization, peak RSS, derived storage sizes, throughput,
//! and cancellation latency as versioned JSON:
//!
//! ```bash
//! cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
//!   baseline /path/to/reference.dsl > reference-pipeline.json
//! ```
//!
//! Its `protocol-selection` command executes the graph in isolated child processes with forced indexed
//! and packed-stream inputs. It verifies the complete writer manifest and immutable Parallel Decoder
//! lane fingerprint before reporting either wall time, so a faster but semantically different path is
//! never accepted as a protocol-selection result:
//!
//! ```bash
//! cargo bench -p logic-analyzer-examples --bench compiler_capture -- \
//!   protocol-selection /path/to/reference.dsl
//! ```
//!
//! Its `live-viewer-runtime` command instead runs a headless production egui viewer during decoding.
//! It reports bounded lane-query and pointer-input frame latency, including counts above the 8 ms and
//! 16 ms foreground budgets. These measurements diagnose presentation stalls and do not participate
//! in the throughput acceptance threshold.
//!
//! The reference workload is the complete approximately 250-second, 50-MHz capture. Its end-to-end
//! throughput acceptance threshold is at least 6x real time (approximately 42 seconds) in the native
//! headless command, with deterministic output, bounded memory, durable cache publication, and
//! responsive cancellation. Compiler-capture results remain useful component measurements but do not
//! satisfy that end-to-end threshold on their own.
//!
//! Correctness tests compare indexed, packed, sequential, and parallel paths, including every
//! strobe mode, CS/enable boundaries, partial-word assembly, deliberately reordered completion,
//! and stop latency.
//!
//! ## Flow control philosophy
//!
//! Minimal buffers, real backpressure. If a consumer is slow, its producer — and every branch
//! sharing that producer — genuinely slows down. That is correct behavior, not a bug, even
//! when it paces an otherwise-independent branch: a truncated or silently lossy decode is
//! worse than a slow one.
//!
//! - Default buffer sizes are small and sized by *item characteristics* (block ≈ 2 MB → 4;
//!   raw RLE edges → millions for burst headroom; sparse events → 100), never to absorb
//!   inter-branch skew.
//! - When a branch must be deliberately decoupled from a slower sibling (e.g. a decoder
//!   feeding both a file writer and the viewer), the mechanism is an explicit **`Buffer`
//!   node** (`BufferNode<T>`, [buffer/mod.rs](../../../crates/logic_analyzer_processing/src/nodes/logic/buffer/mod.rs))
//!   with a user-visible, user-configured capacity — not a bigger invisible default.
//! - `DerivedDataCollector` drains its lanes in bounded batches, so a producer that outruns the viewer
//!   really does block on `send()` rather than the sink racing to keep the channel empty.
//! - The watchdog (below) is the diagnostic net for genuinely-too-small buffers.
//!
//! ## Live supervision
//!
//! Live capture must keep running while the graph is edited. The offline `Pipeline::build`
//! forgets its endpoints; the live path inverts ownership so partial change is possible.
//!
//! ### `SharedSenders` ([sender.rs](../../../crates/signal_processing/src/sender.rs))
//!
//! Two broadcast flavors coexist. Offline: static destination lists moved into threads. Live:
//! a supervisor-owned subscriber list per output — a node thread exiting does *not* close
//! downstream channels; subscribers can be added/removed mid-stream; **sticky level priming**
//! replays the last value (re-stamped) to late joiners. Each subscription carries an
//! `OverflowPolicy`:
//!
//! - `Block` — lossless flow control; the default for every edge, viewer taps included.
//! - `Lossy` — never block; coalesce to the newest value. Suitable only for pure-display
//!   level taps; not used by default.
//! - `Disconnect(deadline)` — block up to the deadline, then unsubscribe the laggard and
//!   report it (`take_disconnected`) so the editor can badge the branch instead of silently
//!   corrupting a live capture.
//!
//! ### `PipelineManager` ([manager.rs](../../../crates/signal_processing/src/manager.rs))
//!
//! Owns, per running node: its thread, its state, a control channel, its output subscriber
//! lists, and its input subscription ids. Operations, cheapest first:
//!
//! | Operation | Mechanism |
//! |---|---|
//! | **Add a tap** | Materialize the new nodes; subscribe their receivers into existing lists; sticky levels prime them. Untouched nodes never notice. |
//! | **Remove a branch** | Unsubscribe its roots, close its own lists → the ordinary shutdown cascade, confined to the branch; join its threads. |
//! | **Reconfigure (hot)** | `NodeCommand::Configure` on the control channel, checked by the scheduler loop between `work()` calls → `apply_config`. |
//! | **Restart in place** | Unsubscribe the node's inputs → its next `recv()` sees `Shutdown` → thread exits cleanly (the normal unwind is the kill mechanism). A fresh instance is spawned with new subscriptions and the *same* output lists (generation + 1). Downstream just sees a quiet channel. |
//!
//! Resynchronization after a restart is each node's normal startup behavior: decoders wait for
//! the next protocol boundary, gates/latches get primed levels. The shared timestamp domain
//! keeps a rejoined branch time-aligned — correct from now on, never misaligned. A node
//! exiting *naturally* closes its own lists on the way out, so end-of-run propagates exactly
//! like the offline drop-cascade. Deferred start (`add_node_deferred` + `start_all_deferred`)
//! lets sources snapshot complete subscriber lists before the first block. The manager also
//! accumulates per-node progress counters (items returned by `work()`, kept across
//! restarts-in-place) that the UI draws in node headers.
//!
//! ### `CooperativeManager` ([cooperative_manager.rs](../../../crates/signal_processing/src/cooperative_manager.rs))
//!
//! Single-threaded sibling for `wasm32` (no `std::thread`). Drives the same `NodeSpec`s and
//! subscriber-list machinery — live add/remove/restart/reconfigure and sticky priming behave
//! identically — but never blocks: `pump(budget)` only calls a node's `work()` when every input
//! is ready **and** no output would block (`SharedSenders::would_block`). Interactive callers use
//! `pump_for(budget, duration)` so the cooperative run returns after a short host-time slice even when
//! the call-count budget has not been exhausted. A blocked-downstream node is skipped for that cycle and
//! retried once the consumer drains. `WorkOutcome::made_progress` keeps the pump running when a
//! node consumed input but deliberately produced no output; the independent produced-item count
//! continues to drive node-header counters. This relies on one invariant: on the cooperative backend
//! a node performs at most one send per output per `work()` call.
//!
//! ## Node library
//!
//! | Category | Nodes |
//! |---|---|
//! | Sources | `DslFileSource` (file replay; per channel `d{i}` edges + `b{i}` blocks), `SigrokFileSource` (native file replay and the explicit embedded synthetic-capture mode), `DsLogicU3Pro16Source` (live USB capture; a `ProcessNode` facade over the driver-neutral `LogicAnalyzer`/`LogicCaptureConfig` interface — see [DSLogic U3Pro16 protocol](../../integrations/dslogic_u3pro16_protocol.md)) |
//! | Decoders | `SpiDecoder` (modes 0–3, 1–64-bit words, MSB/LSB, CS polarity; independent MOSI/MISO words plus CS-bounded transaction packets with incomplete-transfer diagnostics), `I2cDecoder` (address/data direction, ACK/NACK, and repeated-start protocol packets), `ParallelDecoder` (strobe SDR/DDR/level, 1–64 data bits, optional CS/Enable, multi-cycle word assembly with endianness), `UartDecoder` (single-line, derived bit clock, parity/framing error triggers) |
//! | Logic / control | `WordMatcher` (pattern & mask, comparison ops, trigger at word start/end), `EdgeDetector` (rising/falling/both with debounce and preceding-pulse qualification), `EventGate` (signal-level gating of triggers), `EventControl` (delay, holdoff, and optional trigger rearm), `SrLatch`, `LogicGate` (NOT/AND/NAND/OR/NOR/XOR/XNOR over variadic inputs), `TriggerCounter`, `TextFormatter` (template substitution, up to 4 inputs merged in timestamp order), `BufferNode` (explicit decoupling) |
//! | Sinks | `BinaryFileWriter` (filename-level–driven file rollover; never blocks on `Filename`), `TextFileWriter`, `TgckRecorder` (per-capture line-boundary CSV), `DerivedDataCollector` (pushes lanes into the shared `DerivedLanes` store; see [Logic Analyzer Viewer Design](../../crates/logic_analyzer_viewer.md)) |
//!
//! Native binary, CSV-word, and text file sinks access files through the processing-owned
//! `OutputStorage` contract. The production adapter owns directory creation and file open/append
//! semantics. Sink logic receives only writable handles, allowing deterministic storage failures and
//! in-memory output verification without changing rollover or batching behavior.
//!
//! Two merge disciplines exist, chosen deliberately per node:
//!
//! - **Strict timestamp merge** (`LogicGate`): keeps every input's current level plus one
//!   pending edge, applies the globally earliest edge, and blocks on an input whose next edge
//!   is unknown. Safe for levels (an input either advances or closes) and *required* for
//!   correctness: input arrival skew is unbounded, and an event-driven merge would consume the
//!   fast input far past the slow one and corrupt the output timeline. Cost is lag, not
//!   deadlock — the accepted-lag model.
//! - **Event-driven merge** (`SrLatch`): processes whichever input has data, in
//!   `(timestamp, input)` order among what's available. A strict merge would starve here —
//!   sparse trigger streams carry no "nothing happened" information, so a set couldn't be
//!   emitted until the *next* reset arrived. A late-arriving opposite event is clamped to the
//!   last emitted timestamp and logged (defensive; protocol-scale gaps make it practically
//!   impossible).
//!
//! Both stateful level nodes emit their initial state at t=0 (the level contract).
//!
//! ## Observability
//!
//! Structured logging via `tracing`; every node logs under its module path, so `RUST_LOG`
//! filters per node type:
//!
//! ```bash
//! RUST_LOG=info                                          # everything at info
//! RUST_LOG=info,logic_analyzer_processing::nodes::decoders::spi_decoder=debug  # one decoder
//! RUST_LOG=info,logic_analyzer_processing::nodes=debug                         # all concrete nodes
//! ```
//!
//! The **watchdog** ([watchdog.rs](../../../crates/signal_processing/src/watchdog.rs)) monitors
//! blocking channel operations transparently: enabled via `Pipeline::with_watchdog()` (and
//! always on in the managed/live path), it wraps every port so `recv`/`send` register guarded
//! operations, and reports any operation blocked longer than ~5 s with node name, port, and
//! direction:
//!
//! ```text
//! WARN signal_processing::watchdog: ⚠️  BLOCKED: [spi_decoder] recv on port 'clk' for 5.2s
//! ```
//!
//! This pinpoints which node of a stalled pipeline is stuck, on which port — the safety net
//! for the minimal-buffer philosophy above. Node implementations need no watchdog code; the
//! ports handle it.
//!
//! ## Waveform index
//!
//! The finite and growing capture mipmap index (`waveform_index/`), the authoritative artifact-backed
//! capture store (`live_capture_store/`), and the incremental derived-lane index (`derived_index.rs`) are documented in
//! [Logic Analyzer Viewer Design](../../crates/logic_analyzer_viewer.md) — they live in this
//! crate but exist to serve the viewer.

//! UI-independent signal-processing runtime and capture infrastructure.
//!
//! This library provides a memory-efficient node runtime for processing captured and live
//! signals. Concrete logic-analyzer sources, processors, and sinks live in
//! `logic-analyzer-processing`.
//!
//! # Architecture
//!
//! - **Capture contracts**: Generic interfaces for sampled and indexed signals
//! - **Streaming Nodes**: Thread-per-node execution with crossbeam channels
//! - **Scheduler**: Manages node lifecycle and parallel execution
//! - **Derived data**: Generic viewer-lane storage and queries

#[cfg(test)]
mod architecture_tests;
#[cfg(test)]
mod wasm_store_tests;

mod advanced_trigger;
mod app_manager;
pub mod capture;
mod capture_index_kernel;
mod capture_policy;
mod cooperative_manager;
mod crc32c;
mod derived_data_collector;
mod derived_index;
pub mod derived_word_store;
mod edge_query;
mod errors;
mod events;
mod graph;
pub mod live_capture;
pub mod live_capture_store;
pub mod logic_analyzer;
mod manager;
mod node;
mod payload;
mod pipeline;
mod ports;
mod protocol;
mod protocol_packet_payload;
mod receiver;
mod recorded_edge_query;
mod sample;
mod sample_kind;
mod sampling_points;
mod scheduler;
mod sender;
mod storage;
mod time_source;
mod type_registry;
mod watchdog;
pub mod waveform_index;
mod work_executor;
mod worker_kernels;
mod worker_operation_queue;

pub use advanced_trigger::{
    RegisteredTriggerPredicateSchema, TRIGGER_PROGRAM_FORMAT_VERSION, TriggerChoice, TriggerCount,
    TriggerCountCapabilities, TriggerCountMode, TriggerEditorSchema, TriggerIdentifier,
    TriggerLogicOperator, TriggerOperandKind, TriggerOperandSchema, TriggerOperandValue,
    TriggerPredicate, TriggerProgram, TriggerProgramEditError, TriggerProgramForm, TriggerStage,
    TriggerValidationCode, TriggerValidationDiagnostic, TriggerValidationErrors,
    ValidatedTriggerProgram,
};
pub use app_manager::{
    AppManager, AppManagerBackend, AppManagerFactory, CooperativeAppManagerBackend,
    CooperativeAppManagerFactory,
};
pub use capture::{
    BlockCaptureSource, BlockData, CaptureDataSource, CaptureFingerprint, CaptureIndex,
    CaptureIndexBuildProgress, CaptureIndexFactory, CaptureIndexOpenStep, CaptureIndexOpenTask,
    CaptureIndexPreparationRequest, CaptureIndexProxy, CaptureIndexQuery,
    CaptureIndexQueryExecutor, CaptureIndexQueryUpdate, CaptureMetadata, CaptureSampledChannel,
    CaptureSampledWindow, CaptureSampledWindowPoll, CaptureSource, CaptureTransition,
    CaptureWaveformSegment, CaptureWorkerClient, CaptureWorkerIndexQueryExecutor,
    CaptureWorkerMessage, CaptureWorkerOperationRegistry, CaptureWorkerPreparedIndex,
    CaptureWorkerReplayBlock, CaptureWorkerReplayRequest, CaptureWorkerReplaySource,
    CaptureWorkerRequest, CaptureWorkerRuntime, IndexedCapturePresentation,
    decode_capture_worker_messages, encode_capture_worker_messages, packed_bit,
};
pub use capture_policy::{
    CaptureFraction, CapturePolicy, CapturePolicyCapabilities, CapturePolicyContext,
    CapturePolicyError, CaptureRetentionPin, CaptureRetentionTracker, CaptureSessionPlan,
    CaptureStartMode, CompletionPolicy, CompletionPolicyKind, EffectiveCapturePolicy,
    RecordingStart, RetentionPolicy, RetentionPolicyKind, TriggerPlacement,
    TriggerPlacementCapability, TriggerTimeout, TriggerTimeoutAction,
};
pub use cooperative_manager::CooperativeManager;
pub use derived_data_collector::{
    CollectedWordLaneOptions, CollectedWordLaneQuery, DEFAULT_DERIVED_DATA_MAX_ENTRIES,
    DerivedDataCollector, DerivedDataCollectorMetrics, DerivedDataCollectorMetricsSnapshot,
    DerivedDataRetention, DerivedLanes, DigitalLaneSnapshot, IndexedAnnotationLane,
    NumberLaneSnapshot, OpaqueCollectedLane, TextLaneSnapshot, TriggerLaneSnapshot,
    WordLaneSnapshot, built_in_word_lane_ingestor, digital_payload_adapter, number_payload_adapter,
    text_payload_adapter, trigger_payload_adapter, word_payload_adapter,
};
pub use derived_index::{AppendOnlyMipmap, ChunkedMipmap, LaneFold, MipmapRecord};
pub use derived_word_store::{
    AnnotationQuery, BlockCodecConfig, DecodedBlockCacheStats, IndexedAnnotationStore,
    IndexedAnnotationWriter, LiveStoreConfig, PersistentStoreConfig, StoreStatus,
    WordPresenceBucket, cleanup_cache, clear_cache, clear_cache_entry,
    configure_decoded_block_cache, decoded_block_cache_stats, reset_decoded_block_cache_stats,
};
pub use edge_query::EdgeQuery;
pub use errors::{ConnectionError, Error, PortError, Result, WorkError, WorkResult};
pub use events::{
    Annotation, MAX_ANNOTATION_NS, NumberSample, ProtocolPacket, ProtocolValue, TextSample,
    TimelineMarker, Trigger, Word, WordPayload, instantaneous_word_end_ns,
};
pub use graph::{Connection, GraphBuilder, NodeId};
pub use live_capture::{
    AcquisitionContext, AcquisitionError, AcquisitionOutcome, AcquisitionResult,
    CAPTURE_CHUNK_FORMAT_VERSION, CaptureAcquisitionPhase, CaptureAnalysisChannel,
    CaptureAnalysisSource, CaptureBufferLease, CaptureBufferPool, CaptureBufferPoolError,
    CaptureBufferPoolMetrics, CaptureBytes, CaptureChannelId, CaptureChunk, CaptureChunkError,
    CaptureChunkPayload, CaptureChunkWriter, CaptureCommandCapabilities, CaptureCompletion,
    CaptureDataDelivery, CaptureEvent, CaptureEventPublishError, CaptureEventPublisher,
    CaptureEventQueuePublisher, CaptureEventQueueReader, CaptureFailure, CaptureFailureKind,
    CaptureHealth, CaptureProgress, CaptureProviderCapabilities, CaptureQueueConfigError,
    CaptureQueueLimits, CaptureQueueReader, CaptureQueueReceiveError, CaptureQueueWriter,
    CaptureSessionId, CaptureSessionState, CaptureSettingCombination, CaptureStatus,
    CaptureWriteError, ConfiguredAcquisition, PreparedAcquisition, SimpleTriggerCondition,
    bounded_capture_event_queue, bounded_capture_queue,
};
pub use live_capture_store::*;
pub use manager::{DisconnectEvent, InputSub, NodeFailure, NodeSpec, PipelineManager};
pub use node::{
    ConfigOutcome, ConfigValue, ConfigurationBoundary, ConfigurationScheduler,
    InputProtocolCandidate, NodeCancellation, NodeConfig, ProcessNode, RuntimeExecutionMode,
    WorkOutcome,
};
pub use payload::{
    CollectedLaneIngestor, CollectedLaneQuery, CollectedLaneRequest, CollectedLaneSnapshotRequest,
    CollectedLaneStorageBacking, CollectedLaneStorageSnapshot, CollectedLaneTableMetadata,
    CollectedLaneTableRow, CollectedLaneTableSnapshot, OpaqueCollectedLaneSnapshot, PayloadAdapter,
    PayloadDescriptor, PayloadRegistrationError, PayloadRegistry,
};
pub use pipeline::Pipeline;
pub use ports::{InputPort, OutputPort, PortDirection, PortSchema, register_type};
pub use protocol::ProtocolKind;
pub use protocol_packet_payload::{ProtocolPacketLaneSnapshot, protocol_packet_payload_adapter};
pub use receiver::{Receiver, ReceiverSelector};
pub use recorded_edge_query::RecordedEdgeQuery;
pub use sample::{Sample, SampleBlock};
pub use sample_kind::SampleKind;
pub(crate) use sample_kind::negotiate as negotiate_sample_kind;
pub use sampling_points::{
    PackedSamplingPoint, SamplingPoint, SamplingPointProvider, SamplingPointStore,
};
pub use scheduler::{Scheduler, StopHandle};
pub use sender::{ChannelMessage, OverflowPolicy, Sender, SharedSenders};
pub use storage::{
    ArtifactByteSource, ArtifactKey, ArtifactMetadata, ArtifactNamespace, ArtifactReplicationEvent,
    ArtifactReplicationReceiver, ArtifactRepository, ByteRange, ByteRegion, ChunkedByteSource,
    ImmutableByteRegion, MemoryArtifactRepository, OwnedByteSource, PreparedByteSource,
    RandomAccessReader, ReadArtifact, ReplicatingArtifactRepository, RepositoryCapabilities,
    RepositoryError, SourceCapabilities, SourceIdentity, SourceReadError, WriteArtifact,
    read_artifact_region,
};
pub use time_source::{SystemUnixTimeSource, UnixTimeSource};
pub(crate) use watchdog::OperationGuard;
pub use watchdog::{Watchdog, WatchdogHandle};
pub use waveform_index::{
    CaptureIndexProgress, GrowingCaptureIndex, GrowingCaptureIndexWorker, IndexSampler,
    exact_window_sample_limit,
};
pub use work_executor::{
    CompletedWorkTask, CooperativeWorkerOperationExecutor, InlineWorkExecutor, WorkExecutor,
    WorkExecutorTask, WorkTask, WorkerExecutionCapability, WorkerExecutionMode,
    WorkerKernelRegistry, WorkerMessage, WorkerMessageError, WorkerOperation,
    WorkerOperationExecutor, WorkerRequest,
};
pub use worker_kernels::portable_worker_kernels;
pub use worker_operation_queue::{WorkerHostCommand, WorkerOperationQueue};
