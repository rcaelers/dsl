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
transport failures before the graph materializer retains them as typed construction sources.
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
The reusable browser worker adapter separately types JavaScript message validation, request
submission stages, and missing worker slots internally. Those errors become display text only when
the adapter creates the portable serialized `WorkerFailure`; worker-reported host and kernel
messages are already diagnostics received across that wire boundary.
Browser artifact-repository opening distinguishes invalid roots, host persistence-worker stages,
unavailable durable storage, invalid initialization responses, and session hydration through
`platform::ArtifactRepositoryOpenError`. Hydration retains the portable `RepositoryError`; web
composition formats the typed failure only when reporting why it selected the memory repository.
Initialization protocol failures likewise retain their concrete browser validation source.
Persistence command construction and submission, runtime message properties, completion metadata,
and identity decoding use typed private errors until the adapter emits its terminal durability
warning.
Generic native USB opening distinguishes a complete selector miss from context, enumeration,
descriptor, device, identity, configuration, and interface failures through
`platform::UsbDeviceOpenError`. Host failures retain their `rusb::Error`. Driver-neutral
`LogicAnalyzerError::Transport` and session-neutral `AcquisitionError::Transport` retain boxed typed
sources. Native composition injects the platform error, and DSLogic device construction moves that
source between the generic facades without formatting it; providers exposing only diagnostics use
explicit message adapters.

`logic_analyzer_graph_orchestration` owns
separate graph-worker codec, bounded-client, and serializable transport failures. The browser host
retains those categories through disconnect and terminal messages; the UI graph-run adapter formats
them only when projecting its presentation-facing diagnostic. `logic_analyzer_capture_export`
likewise preserves export metadata, capture consistency, capture-store, destination, archive, and
cancellation causes.
Its service retains capture-store and executor sources, worker loss, and the typed exporter failure;
UI policy therefore matches a cancellation variant rather than display text.

The `logic_analyzer_ui` plugin-panel facade classifies invalid and duplicate definitions with
`PluginPanelRegistrationError`. `UiPanelRegistration::validate` exposes that contract to plugin
owners, and inventory assembly carries it to application construction. A plugin returns
`PluginPanelStateError` when persisted state cannot be restored; the error retains a typed
plugin-owned source through panel lookup and is formatted only when the application emits a toast.

Driver-neutral setting combinations, provider capabilities, and analysis sources return the
classified `CaptureValidationError`. Invalid acquisition requests retain that typed source through
`AcquisitionError`, while explicit message adapters remain available to providers that expose only
diagnostics. Analysis-source construction then crosses the graph feature port as the source-bearing
`CaptureGraphSourceError`; concrete graph nodes and the generic capability crate do not format the
session-owned validation cause.

`logic_analyzer_ui` carries live-capture failures through `CaptureCoordinatorError`. Its variants
retain repository, capture-store, graph-source, waveform-index, executor, export, acquisition,
capture-policy, and metadata-codec causes through worker completion, live attachment, finalized
replay, retention, and publication. Status projection and application toast/run-message calls are
the only points that convert this workflow error to text.

`logic_analyzer_graph_capabilities::node_support` owns `PersistedStateError`, retaining JSON decode
and encode causes at the graph-document boundary. Timeline node features carry that error through
`TimelineFeatureError`, alongside classified marker and reference-edit failures. The graph compiler
adds owner-node and operation context through `TimelineOperationError`; UI timeline synchronization
formats the error only for deduplication and presentation.

Live-capture feature discovery, trigger configuration, and document edits use
`LiveCaptureFeatureError` to retain persisted-state and capture-metadata causes and to classify
configuration, edit, and provider-contract failures. `LiveCaptureOperationError` adds graph owner,
registry, ambiguity, and generic provider-validation context. UI availability, trigger status, and
toast handling are the formatting boundaries.

Runtime node construction uses `RuntimeMaterializationError`. Persisted-state failures and typed
factory failures remain error sources; node-owned configuration, missing run resources, legacy
construction diagnostics, and invalid capability paths remain separately classifiable. The graph
runtime adds node or pipeline context through `GraphRuntimeError`, and live reconciliation carries
the same materialization source through `ApplyError`. UI diagnostic projection and graph-worker
serialization are the text-formatting boundaries.

Portable writer factories use `WriterConstructionError` for configuration, typed construction, and
explicit diagnostic adapters. DSL and Sigrok factories use `CaptureSourceConstructionError` to
retain prepared-source access and capture-format failures. The DSLogic source factory uses
`DsLogicU3Pro16SourceError` to retain driver-neutral acquisition failures. Graph materializers wrap
these owner errors as typed construction sources; native, browser, test, and benchmark factories
implement the same portable contracts.

Generated collector request customization returns the plan-owned
`PayloadCatalogConfigurationError`; the registry retains missing-subscription context through
`PayloadRequestConfigurationError`, and the compiler preserves that typed cause in its immutable
catalog adapter. `signal_derived::PayloadAdapter` returns `PayloadIngestorConstructionError` for
invalid requests, typed adapter failures, and explicit diagnostic adapters. Graph runtime adds
collector member, adapter, and lane context without formatting either owner error.

The portable Sigrok execution facade separates worker startup from running lifecycle failures.
`SigrokExecutionStartError` retains typed factory-start causes, and `SigrokExecutionError`
classifies input submission, output retrieval or conversion, completion, and join failures. The
native Python adapter preserves executor, bridge, worker, and `PyErr` sources; the portable decoder
retains that execution error through `WorkError::NodeSource`, and `NodeFailure` carries it through
generic runtime supervision.

Sigrok package discovery uses `SigrokDecoderDiscoveryError` to distinguish unavailable hosts,
Python package inspection, package fingerprinting, and diagnostic-only adapters.
`SigrokDecoderRuntimeError` retains that source. Native inspection preserves `PyErr`, while package
enumeration and content hashing preserve the originating filesystem error. Catalog snapshots keep
missing or unreadable paths and invalid individual packages as recoverable structured diagnostics;
`SigrokCatalogError` retains a typed source only when a scanner cannot produce a snapshot at all.

Native Sigrok directory settings persistence returns `SigrokCatalogSettingsError`. It retains
platform document causes for reads, parent-directory creation, and writes, and preserves JSON
decode and encode causes. A missing settings file remains the ordinary first-run state. Persistence
diagnostics are stored separately from discovery diagnostics and are formatted only when the
application builds the node-catalog snapshot.

Portable trigger schema construction returns `TriggerSchemaError`; identifier, range, default,
uniqueness, limit, and simple-program representability failures no longer cross crate boundaries as
strings. `TriggerProgramEditError` retains schema and program-validation causes. The independent
trigger widget classifies reducer failures through `TriggerEditorError` and formats them only when
building its presentation response. Graph trigger assembly uses `TriggerConfigurationError` for
duplicate channel mappings and invalid programs, and `LiveCaptureFeatureError` retains that source
through feature discovery.

The reusable node-graph widget returns `GraphSnapshotError` when synchronizing its editor state to
a JSON document snapshot. The error retains `serde_json::Error`; application composition formats it
only for save/status presentation, while host document encoding remains the separate UI-owned
`GraphDocumentError` boundary.

Capture-worker operation inventory construction returns
`CaptureWorkerOperationRegistrationError`, distinguishing duplicate stable identifiers without
turning them into diagnostics. Preparation returns `CaptureWorkerOperationPreparationError`, which
separates missing handlers from registered handler failures and retains the adapter-owned error as
its source. The runtime formats that local failure only when constructing the serializable
`CaptureWorkerFailure` terminal message. Browser capture preparation supplies a typed JSON or range
failure through the same handler contract.

UI presentation-catalog assembly returns `PresentationBindingError`. Its variants distinguish a
payload missing its required default lane presentation from absent waveform-lane and decoder-table
renderer registrations, retaining the relevant stable payload, lane, column, and renderer keys.
The application toast projection is the boundary that formats those catalog contract failures.

Browser capture-worker composition returns `BrowserCaptureWorkerInstallError`, retaining capture-
and graph-client configuration causes while classifying bootstrap, worker startup, initialization,
URL cleanup, window availability, and pump startup. `BrowserWorkerMessageError` owns JavaScript
message property access, writes, and type validation. File submission and asynchronous completion
share `BrowserCaptureAttachmentError`, which preserves message-shape and metadata codec causes and
separates submission, invalid identity, and worker-reported failures. The web application logs an
installation failure only when selecting its inline fallback; the file-picker adapter formats an
attachment error only when constructing its presentation-facing import failure.

The browser imported-file registry returns `BrowserFileRegistryError`. It distinguishes per-file
limits, address-space overflow, session-budget exhaustion, reference exhaustion, duplicate worker
references, and unavailable saved references while retaining `SourceReadError` from resident chunk
validation. DSL and Sigrok browser adapters carry registry lookup failures as typed metadata-access
or source-construction causes. Only the file-picker adapter formats registration failures into its
presentation-facing import result.

The graph worker's browser-file source facade returns `BrowserWorkerSourceError`. It distinguishes
malformed preparation references, JavaScript length limits, capture-metadata parsing, and missing
worker-cache entries. Metadata parsing retains the generic capture error, while DSL and Sigrok
worker factories preserve cache lookup failures through capture-source metadata and construction
errors. The wasm export formats this error only when returning the final JavaScript diagnostic.

The combined browser worker adapter returns `BrowserWorkerTransportError` while validating inbound
messages and submitting outbound capture and graph requests. It retains the primary neutral worker
transport failure, JavaScript message classification, request channel, and graph-output JSON cause.
Conversion to a string occurs only for the serializable `Host` variant delivered to the other
worker client, or for the terminal graph-output warning. The browser `HostService` adapter needs no
parallel error enum: it already maps typed platform document and download failures directly into
the UI-owned contracts.

**How to type an error here** (`thiserror` is already a workspace dependency):

- One enum per *facade*, not per crate and not per function. Variants describe what failed in
  the owner's vocabulary; a wrapped lower-level error rides in a variant field.
- Convert at ownership boundaries: a crate maps a dependency's error into its own variant
  (`#[from]` only when the dependency type is itself part of the crate's contract). Never
  `format!` a message and pass it up — formatting happens once, at the presentation boundary.
- `Result<_, String>` in *tests* is fine; do not churn test code.

**Order (work outward from the lowest owner, per the TODO item):**

1. Type Sigrok package discovery and catalog scanning so filesystem enumeration and Python package
   inspection retain their causes through `SigrokDecoderRuntimeError` and `SigrokCatalogError`.
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
