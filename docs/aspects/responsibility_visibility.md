# Responsibility and Visibility Design

## Design

Every module and crate exposes only capabilities that belong to its stated responsibility.
Visibility is an architectural contract: an item is public only when a consumer at that
visibility boundary is expected to depend on it.

Crate APIs use responsibility-oriented names and errors. Generic crates do not retain aliases,
error variants, helpers, or dependencies for a concrete capture format, device, protocol, node,
or presentation. Compatibility for a concrete feature is owned by that feature's crate and is
made explicit at its load or migration boundary.

## Ownership

The crate boundaries in `AGENTS.md` are enforced at both dependency and symbol level:

- `platform_artifacts` owns immutable byte regions, prepared sources, artifact identities,
  repository contracts, replication, and shared persistence primitives.
- `platform_runtime` owns generic host work scheduling, worker-operation messages, kernel
  registration, and portable worker-queue policy.
- `signal_runtime` owns generic typed-stream execution, scheduling, and pipeline supervision.
  Its root also owns the generic `ProcessNodeConstruction<M>` result contract and the cloneable
  source-bearing node-work failure boundary.
- `signal_capture` owns immutable generic capture, query, and finite indexing contracts.
- `signal_derived` owns generic derived-data payload, collection, indexing, storage, the explicitly
  injected decoded-block cache handle, and typed payload-ingestor construction failures.
- `signal_capture_session` owns generic capture-session
  contracts and lazy capture-source metadata/lifecycle contracts. Its validation facade classifies
  invalid setting matrices, provider capabilities, and analysis-source layouts; invalid acquisition
  requests retain those typed causes. It consumes fixed-width byte ranges, stable source identities, prepared
  random-access sources, immutable byte regions, and portable memory sources directly from
  `platform_artifacts` without re-exporting them. Host paths, files, mappings, and browser handles
  are absent from those contracts. Its public capture
  vocabulary is `Capture*`; it does not expose DSL, Sigrok, USB, decoder, graph-node, or UI
  terminology.
- `logic_analyzer_trigger` owns portable trigger programs, schemas, predicates, simple digital
  conditions, typed schema-construction and edit failures, and validation diagnostics.
- `logic_analyzer_acquisition` owns shared device-neutral driver, capture-configuration,
  hardware-trigger, raw-chunk, and runtime-source contracts.
- `logic_analyzer_capture_formats` owns DSL and Sigrok parsing, indexing, replay sources, and their
  typed construction boundary.
- `logic_analyzer_device_dslogic` owns DSLogic acquisition, its injected transport contract, and
  typed source construction failures.
- `logic_analyzer_protocol_decoders` owns concrete protocol decoding, decoder host contracts, and
  typed Sigrok execution startup and lifecycle failures.
- `signal_transforms`, `signal_sinks`, and `signal_generators` own portable stream transforms,
  terminal consumers, and deterministic configured sources respectively. `signal_sinks` also owns
  the shared typed writer-construction boundary.
- `logic_analyzer_graph_capabilities` owns graph-node and payload capability contracts. Its capture
  graph-source factory carries a source-bearing construction error without interpreting the
  session-owned validation cause. Its persisted-state facade retains JSON decode and encode causes,
  and timeline capabilities classify state and edit failures without formatting them.
- `logic_analyzer_graph_registry` owns graph-node and payload registration, inventory validation,
  immutable catalog assembly, and typed payload-request configuration failures.
- `logic_analyzer_graph_nodes` owns built-in concrete node definitions, builders, migrations,
  registrations, and presentation metadata.
- `logic_analyzer_graph_compiler` owns generic graph lowering, document discovery, and semantic
  diagnostics.
  Definition defaults and lowering helpers remain crate-private unless plugin authors or another
  crate implement against a documented contract.
- `logic_analyzer_graph_plan` owns the immutable processing-plan contract exchanged between plan
  producers and consumers, including the typed payload-catalog configuration boundary.
- `logic_analyzer_graph_runtime` owns materialization, source preparation, cache planning,
  execution lifecycle, collected run data, and live reconciliation.
- `logic_analyzer_graph_orchestration` owns the graph-worker protocol and worker-side composition
  above a separate compiler and runtime.
- `node_graph_document` owns graph records, identities, neutral presentation values, serialization,
  and document-local invariants. Headless graph crates depend on this owner directly.
- `node_graph::api` owns node-definition and editor integration contracts. Graph-node definition
  code depends on this namespace; widget and editor operations remain at the `node_graph` crate
  root for UI composition. File controls depend on its portable
  `FileDialogService`; its `FileDialogError` retains an injected host cause without depending on a
  platform crate. The widget defaults to an unavailable implementation and application composition
  injects the host adapter.
- `logic_analyzer_capture_export` owns native streaming export of finalized generic capture
  storage plus the stateful export-service contract and asynchronous native implementation. It
  depends on capture contracts and format libraries, not graph crates, UI, platform, or concrete
  processing nodes.
- `logic_analyzer_test_support` owns deterministic capture providers and data-plane conformance
  fixtures shared by cross-crate tests. It depends on generic runtime contracts rather than
  concrete processing, graph, or UI crates.
- `logic_analyzer_ui` owns the concrete application graph service. Application orchestration
  calls its private `UiGraphService`, which composes `GraphLowerer` and `GraphRuntime` directly;
  the private `GraphRun` trait remains the execution-lifecycle boundary between local `LiveRun`
  and worker-backed runs. UI tests exercise the concrete service with injected repositories and
  executors. Its public `HostService` port owns file and directory dialogs, graph-document
  persistence, derived-cache commands and diagnostics, and native-shell state exchange. Native and
  web application roots implement that application-facing port by adapting low-level host
  mechanisms. Its plugin-panel facade owns registration validation and source-bearing persisted-state
  restoration errors, keeping plugin causes typed until application toast presentation. Its
  live-capture coordinator likewise owns the application workflow error that retains repository,
  capture-store, graph-source, waveform, executor, export, and acquisition causes until status or
  toast presentation. These
  application and domain contracts do not belong to `platform`; it remains
  independent of UI and graph crates. The UI consumes and re-exports the
  `logic_analyzer_capture_export` service contract;
  `CaptureCoordinator` supplies only a finalized session identity and retains capture lifecycle
  policy. Native composition injects the repository-backed exporter, while the UI supplies an
  explicit portable unavailable implementation for hosts without an export destination.
- Workspace-level integration tests own end-to-end compositions spanning concrete graph nodes,
  processing nodes, and the generic compiler. Component crates keep only tests of their own
  contracts and private implementation mechanics; composition-only dependencies do not appear in
  their manifests.
- Generic graph, viewer, compiler, runtime, and widget crates consume explicit metadata and
  capability contracts. They do not infer concrete behavior from names.
- Presentation helpers shared by widgets live in a neutral widget module or crate. Input-binding
  crates expose input and shortcut behavior, not unrelated menu layout policy.

Concrete aliases are declared beside their concrete implementation. A common abstraction module
does not import one implementation merely to publish a convenience alias.

Generic storage accepts explicit working, persistent-cache, and session-repository directories.
Native application composition owns the application namespace and operating-system directory
policy, then passes resolved paths through configuration. Generic crates do not inspect host
environment variables to choose an application location.

## Visibility rules

Use the narrowest visibility that contains every intended consumer:

- private for implementation details used in one module;
- `pub(crate)` for collaboration between sibling modules or an internal crate contract;
- `pub` only for a supported cross-crate or plugin contract.

A `pub` item hidden below a private module is still an invalid declaration unless its wider
visibility is required by a public signature. Public re-exports are deliberate API decisions,
not a convenience for internal imports.

Public traits expose a complete implementable contract. Every type in their required method
signatures is publicly nameable from a stable path. Conversely, implementation seams that are
not supported extension points remain private, including their generic parameters and errors.

## Module layout

The workspace uses an owner-facade module layout.

### Module ownership

A substantial module is an architectural owner inside its crate. A module that exceeds roughly
1,000 lines or owns cross-cutting mutable state states four things in its module documentation:

1. the data and invariants it owns;
2. the facade through which sibling or external consumers use it;
3. the owner-level dependencies it may consume; and
4. the behavior and data it explicitly excludes.

The line threshold is advisory, not a lint target. A shorter stateful module can still require an
ownership statement, and a long cohesive implementation is acceptable when its answer remains
concise. A module that cannot answer the four questions without listing unrelated behaviors is a
decomposition candidate. Extracted owner structs keep their fields private and expose invariant-
preserving methods through a directory-backed facade; a caller passes another owner or narrow view
only when a transition genuinely couples them.

### Source structure

- Module declarations occur only in `lib.rs`, `main.rs`, and `mod.rs` files.
- Test modules are the only exception. They may be declared in any Rust file, but every test
  module name contains `tests`.
- Modules are private by default. An owning `mod.rs` or crate `lib.rs` selectively re-exports the
  symbols that form its internal or external contract.
- Leaf files within the same module use sibling implementation paths directly, such as
  `super::presentation::render` or `super::definition::State`. They do not route same-module
  dependencies through re-exports in their own `mod.rs`; a facade re-export serves only consumers
  outside the owning module.
- A public module is an intentional API namespace, not a way to make its implementation easier
  to import. Public modules are limited to the allowlist below.
- Every public module is directory-backed and has a `mod.rs`; public file modules such as
  `pub mod capture;` backed by `capture.rs` are not permitted.
- A `mod.rs` contains module documentation, attributes on module declarations or re-exports,
  module declarations, and re-exports only. Structs, enums, traits, implementations, functions,
  constants, type aliases, executable macro bodies, and other implementation code live in leaf
  files.
- Target selection groups complete module declarations and re-exports with `std::cfg_select!` in
  the explicitly allowlisted platform and device-adapter selection facades. The macro arms contain
  declarations and re-exports only; inline implementation modules and executable macro bodies do
  not belong in a `mod.rs`. Other modules do not use selection macros.

### Visibility through facades

Leaf symbols used only in their defining file are private. Symbols shared directly between
sibling leaf modules are `pub(crate)` at their definition but are not re-exported by the owning
`mod.rs`. A symbol re-exported for another module in the same crate is `pub(crate)` at its
definition and at the owning facade. A supported cross-crate or plugin contract is `pub` at its
definition and is publicly re-exported from an allowlisted public module or the crate root.

The layout does not use `pub(super)` or `pub(in ...)`. The facade path communicates the
owner and intended dependency direction, while `pub(crate)` provides the visibility required to
form an internal re-export. `pub` never means merely "used by another file"; it always denotes a
supported external contract.

Struct fields are private by default. Behavioral and invariant-owning structs expose methods.
Plain record types intended for construction or pattern matching may expose their data fields,
but those fields use one consistent visibility matching the record contract. A struct does not
mix private, `pub(crate)`, and `pub` data fields; read-only access uses methods instead.

### Public-module allowlist

All modules absent from this table are private and expose supported symbols through their
nearest owning facade. The allowlist names canonical public namespaces.

| Crate | Public modules | Rationale |
| --- | --- | --- |
| `platform_artifacts` | none | Its crate root exposes immutable byte, source, repository, replication, clock, and integrity contracts; implementation modules remain private. |
| `platform_runtime` | none | Its crate root exposes host work, worker-operation, kernel, capability, and queue contracts; implementation modules remain private. |
| `signal_runtime` | none | Its crate root is the curated stream-execution facade; ports, channels, schedulers, and managers remain private implementation modules. |
| `signal_capture` | none | Its crate root exposes immutable capture, query, edge-capability, and finite-index contracts; implementation modules remain private. |
| `signal_derived` | `derived_word_store` | The public module owns the independently usable encoded annotation-store and decoded-block cache contracts; other payload, lane, sampling, index, and typed ingestor-construction contracts are exposed through the crate facade. |
| `signal_capture_session` | `live_capture`, `live_capture_store` | These are substantial generic capture-session domains. `live_capture` owns provider-neutral configured and prepared acquisition contracts; `live_capture_store` owns recording and committed-prefix storage. Lower-level runtime, capture, and derived contracts are imported directly from their owning crates and are not re-exported. |
| `logic_analyzer_trigger` | none | Its crate root exposes serializable trigger programs, schemas, predicates, simple conditions, and validation contracts; implementation modules remain private. |
| `logic_analyzer_acquisition` | none | Its crate root exposes device-neutral driver, capture-configuration, hardware-trigger, raw-chunk, and runtime-source contracts; implementation modules remain private. |
| `logic_analyzer_capture_formats` | `dsl_file`, `sigrok_file` | Each format facade owns its configuration, factory, parser, index, and replay contracts; the crate root exposes their shared typed construction error and archive helpers remain private. |
| `logic_analyzer_device_dslogic` | none | Its crate root exposes the DSLogic source, typed source-construction error, and transport contracts; protocol implementation modules remain private. |
| `logic_analyzer_protocol_decoders` | `i2c_decoder`, `packet_framer`, `parallel_decoder`, `sigrok_decoder`, `spi_decoder`, `types`, `uart_decoder` | Each decoder or protocol-packet processor has one directory-backed public facade; shared packet and decoder conventions live under `types`. |
| `signal_transforms` | `buffer`, `edge_detector`, `event_control`, `event_counter`, `event_gate`, `logic_gate`, `sr_latch`, `text_formatter`, `timeline_marker`, `word_field_extractor`, `word_matcher` | Each namespace owns one portable transform contract and implementation. |
| `signal_sinks` | `binary_file_writer`, `csv_word_writer`, `discard_writer`, `text_file_writer`, `tgck_recorder` | Each namespace owns one sink; the shared destination and typed writer-construction contracts are exposed through the crate root. |
| `signal_generators` | `synthetic_capture_source`, `synthetic_uart_source` | Each namespace owns one explicit deterministic source family. |
| `logic_analyzer_graph_capabilities` | `node`, `node_support` | `node` owns capability traits implemented by graph-node plugins. `node_support` owns open port identity, protocol-neutral presentation descriptions, capture descriptions, decoder-table contracts, and the restricted node build context. It contains no graph-node or payload inventory assembly, compiler, host, built-in-node, UI, or export operations. |
| `logic_analyzer_graph_editor_registry` | none | Its crate root exposes stable-ID-keyed node-editor registration, validated editor inventory access, and instance-bound editor overrides. Implementation modules remain private. |
| `logic_analyzer_graph_registry` | none | Its crate root exposes graph-node and payload registration descriptors, typed request-configuration failures, validated inventory access, and the immutable `GraphRegistry`. Implementation modules remain private. |
| `logic_analyzer_graph_plan` | none | Its crate root exposes the immutable `ProcessingGraph`, processing-node/edge, typed payload-materialization, subscription, sampling, and diagnostic contracts exchanged between compiler and runtime. |
| `logic_analyzer_graph_compiler` | none | Its crate root exposes `GraphLowerer` and document-discovery results. Processing-plan values are imported from `logic_analyzer_graph_plan`, capability contracts from `logic_analyzer_graph_capabilities`, and registry contracts from `logic_analyzer_graph_registry`; the compiler crate does not forward them. |
| `logic_analyzer_graph_runtime` | none | Its crate root exposes `GraphRuntime`, `LiveRun`, run-data and source-preparation results, cache operations, and source-preparation execution contracts. Execution implementation modules remain private. |
| `logic_analyzer_graph_orchestration` | none | Its crate root exposes graph-worker request, message, codec, client, and worker-runtime contracts. Lowering and execution behavior remain in their owning crates. |
| `logic_analyzer_graph_nodes` | none | The crate root exposes the linker anchor, host-injection and portable-template helpers, and the built-in protocol-packet lane snapshot required by domain integration consumers. Built-in graph-node definitions, socket types, migrations, presentations, inventory submissions, and crate-local test fixtures remain private. Cross-component fixtures belong to the top-level integration-test package. |
| `logic_analyzer_capture_export` | none | The cohesive exporter exposes its curated format, progress, observer, report, stateful service contract, and native asynchronous implementation through the crate root. Encoder, archive, and service implementation modules remain private. |
| `platform` | none | The crate root exposes individually scoped target-selected host constructors. Private native and web modules contain the single reusable target-selection point. |
| `logic_analyzer_test_support` | none | Shared deterministic acquisition providers and data-plane conformance fixtures are exposed through the crate root. Their synchronization, repository observation, and fixture implementations remain private. |
| `node_graph_document` | none | Its crate root exposes portable graph records, identities, neutral presentation values, serialization, and semantic socket references. Implementation modules remain private. |
| `node_graph` | `api` | `api` exposes node-definition contracts and compatibility re-exports of document records to graph-node implementations. The crate root exposes the widget/editor composition surface and its typed graph-snapshot serialization failure used by UI hosts. |
| `logic_analyzer_viewer` | none | The reusable viewer exposes one curated crate-root API; drawing, sampling, input, cursor, lane, worker, and indexing modules remain private. |
| `logic_analyzer_ui` | none | The application-composition crate exposes only its host-facing crate-root facade, including portable host services and typed plugin-panel registration and state-restoration contracts. |
| `input_bindings`, `panel_layout`, `trigger_editor`, `widget_support` | none | Each crate represents one cohesive public component and does not need a second namespace level. |
| Native/web application crates and example plugins | none | Binary integration and plugin linker anchors are crate-root entry points; inventory submissions and other implementation modules remain private. |

Changing this allowlist is an API-design decision. A new public module requires a documented
domain boundary, more than import convenience, and review of its native and wasm surface.

Each concrete `logic_analyzer_graph_nodes` node directory owns an isolated registration test. Test-only
wildcard source and sink definitions negotiate the node builder's declared payload kinds, so the
test lowers only that node and does not import concrete neighboring nodes. A concrete graph-node
`mod.rs` does not re-export its definition, state, builder, or other symbols, including under
`cfg(test)`. Multi-node fixture and compiler tests discover nodes through inventory stable IDs and
edit their serialized state through the generic graph contract.

### Enforcement

The source-structure check in CI rejects module declarations outside the
allowed root files, non-test exceptions, test module names without `tests`, public file modules,
implementation items in `mod.rs`, public modules outside the allowlist, and occurrences of
`pub(super)` or `pub(in ...)`. It also rejects symbol re-exports from concrete graph-node facades.
The workspace enables `-D unreachable-pub`.

## Error boundaries

Generic errors describe failures at the abstraction boundary, such as I/O, invalid generic
indices, or malformed generic storage. Concrete parsers and transports own their detailed error
types and dependencies. When they implement a generic source or runtime trait, they translate a
concrete error into a generic boundary error without moving the concrete dependency into the
generic crate.

`platform_runtime` distinguishes work-executor admission, worker-kernel execution, bounded-queue
admission, message validation, and terminal worker failure. Native and browser adapters map their
mechanism errors into those contracts; higher-level runtimes can classify the failure before any
presentation boundary formats it.

`platform` document mechanisms retain native filesystem and browser-session failures in
`DocumentError`. Application roots adapt those mechanisms to the UI-owned `GraphDocumentError`,
which classifies graph read, decode, encode, and write failures without making UI depend on
`platform`. Toast and headless reporting format the error.

`platform::DownloadError` distinguishes expired queued outputs from individual host download
activation stages. Web composition retains that cause in the UI-owned `OutputDownloadError`, and
only output-download presentation formats it.

`platform::WorkerAdapterError` owns host worker-pool construction failures. It retains
`platform_runtime::WorkerQueueError` for portable queue configuration, `std::io::Error` for native
thread creation, and classified browser bootstrap-stage diagnostics. Native composition propagates
the typed source to application startup; web composition renders it only as the cooperative
executor's explicit unavailability reason.

`platform::ArtifactRepositoryOpenError` owns reusable host repository-opening failures. It
distinguishes invalid namespaces, host persistence-worker stages, unavailable durable storage,
invalid initialization responses, and hydration. Hydration retains the lower
`platform_artifacts::RepositoryError`; browser composition formats the typed failure only when
reporting its explicit in-memory fallback.

`platform::UsbDeviceOpenError` owns generic native USB discovery and opening failures. It separates
a complete selector miss from classified context, enumeration, descriptor, identity,
configuration, and interface operations, and host-operation variants retain their concrete
`rusb::Error`. The platform type contains no device-model or protocol knowledge.

`logic_analyzer_acquisition::LogicAnalyzerError::Transport` owns driver-neutral transport failure
propagation, and `signal_capture_session::AcquisitionError::Transport` owns the corresponding
session-lifecycle boundary. Both retain boxed typed sources and expose explicit message adapters for
providers that have only diagnostics. Concrete device adapters move sources between these neutral
facades without depending on their platform type.

`signal_runtime` distinguishes port lookup, connection validation, pipeline construction and
supervision, and process-node work. Threaded and cooperative managers expose the same
`PipelineError` lifecycle contract, and a terminal `NodeFailure` retains its `WorkError` until a
presentation or transport boundary formats it. `WorkError::NodeSource` retains an owner-specific
typed processing failure while the legacy diagnostic variant marks nodes that expose only text.

`signal_capture` owns the capture-worker request and message codec, bounded-client admission and
correlation errors, serializable transport failures, and classified preparation, query, and replay
terminal failures. Capture-operation registration classifies duplicate identifiers, while
preparation distinguishes missing handlers from typed handler-owned causes. Serialization into a
terminal worker failure is the first point where those local causes become transport diagnostics.
Its client retains a typed disconnect cause for pending and subsequent requests.
Graph source preparation wraps the capture-worker client and terminal errors as sources in
`SourcePreparationError`; it does not relabel them as executor or index strings. The neutral
`CaptureIndexQueryExecutor` port reports classified submission, execution, cancellation,
disconnection, and invalid-update failures. Its source-bearing categories retain concrete adapter
errors through `CaptureIndexProxy` without making the generic query contract depend on a worker.

`signal_capture_session` owns lazy capture-source metadata inspection. Its metadata facade
classifies source access, metadata decoding, and live-acquisition configuration separately and
retains typed adapter causes. Providers that expose only diagnostic text use the facade's explicit
message adapter, keeping that loss of source type visible at the provider boundary.

`logic_analyzer_graph_capabilities` maps saved-state and metadata inspection failures into its
generic capture-source feature contract. `logic_analyzer_graph_plan` owns the typed discovery result
exchanged between compiler and runtime, including feature, identity-encoding, and multiple-source
selection failures. The compiler adds graph-node context without formatting the feature cause, and
graph-runtime source preparation retains the complete discovery error.

The same capability crate owns `PersistedStateError` for decoding and encoding node-owned document
state and `TimelineFeatureError` for timeline discovery and edits. Concrete timeline nodes preserve
the JSON codec cause through that feature contract. `logic_analyzer_graph_compiler` adds graph-node
and operation context through `TimelineOperationError`; UI timeline synchronization formats it only
when deduplicating or presenting an error.

Live-capture feature discovery, trigger configuration, and state editing use
`LiveCaptureFeatureError`. It preserves persisted-state and lazy capture-metadata causes and
typed trigger-configuration causes, and classifies node-owned configuration, edit, and
provider-contract failures. The compiler adds graph
ownership, registry, ambiguity, and generic provider-validation context through
`LiveCaptureOperationError`; UI capture availability, trigger status, and toasts are its formatting
boundaries.

`logic_analyzer_protocol_decoders::sigrok_decoder` owns the host-facing Sigrok catalog and decoder
runtime error contracts. Whole-catalog discovery failures are distinct from recoverable per-path
and per-package catalog diagnostics. Decoder runtime failures distinguish package discovery,
portable configuration validation, and host execution startup without exposing PyO3 or another
adapter dependency. Its execution port separately classifies input, output, completion, and join
failures while retaining adapter-owned causes.

`logic_analyzer_graph_runtime` classifies finite-source discovery, metadata inspection, index
construction, cancellation, executor, and worker-protocol failures in `SourcePreparationError`.
Discovery retains the graph-plan error, while index metadata inspection and index construction
retain their `signal_capture::Error` causes. Executor admission and loss retain the neutral
`platform_runtime::WorkExecutorError`; invalid preparation responses retain a runtime-owned
`SourcePreparationProtocolError` with the unexpected response kind.
Its cache-administration facade retains derived-store and host-executor causes in
`DerivedCacheError`; background and cooperative cleanup return the same error contract. UI
presentation formats that error, while worker composition converts it only when building the
serializable graph-worker terminal message.

`logic_analyzer_graph_orchestration` separately classifies codec, bounded-client, and serializable
transport failures. Its client retains the typed transport cause after disconnection and in every
pending run's terminal message. The browser host maps JavaScript mechanism failures into that
contract, while codec and artifact-repository causes stay structured; the UI maps the terminal
failure into its own graph-run presentation contract. `logic_analyzer_capture_export`
retains cancellation separately from unavailable, lifecycle, capture-access, executor, and export
failures across its application-service facade. Its exporter retains typed metadata, capture
consistency, store, destination, and archive failures rather than collapsing them into an early
display string.

`logic_analyzer_ui` owns the presentation-catalog binding between lowered generic metadata and the
registered waveform and decoder-table renderer inventories. `PresentationBindingError`
distinguishes missing default lane metadata from unknown lane and table renderers, retaining their
stable keys until application toast presentation.

The web application root owns its combined capture/graph worker adaptation. Installation retains
the portable capture- and graph-client configuration errors and classifies browser bootstrap
stages. Its message facade validates JavaScript properties, and capture attachment retains message
and metadata causes through asynchronous completion. These application-level adapters compose the
neutral worker clients; they add no worker policy to `platform`.

The same application root owns the session-local imported-file registry. Its error contract
classifies file and session limits, reference lifecycle failures, and resident byte-source
validation. Browser DSL and Sigrok adapters retain lookup errors through the neutral metadata and
source-construction facades rather than exposing registry diagnostics as their domain contract.
The graph worker's browser-file source facade separately classifies preparation-payload decoding,
JavaScript length limits, capture-metadata parsing, and worker-cache lookup. It retains capture
parsing and lookup causes through the neutral metadata and source-construction facades, formatting
them only at the wasm export boundary.

## Platform surfaces

Native and wasm reusable crates share the platform-neutral data model and compile the same source
tree. Native-only filesystem, USB, mmap, worker, export, and host-integration capabilities are
selected as complete adapter modules in `platform`. A platform facade exposes a
complete contract; consumers do not depend on an unnameable backend type or a target-dependent
collection of incidental helpers.

`AppManager` owns one portable facade over an injected `AppManagerBackend`. The native application
combines `PipelineAppManagerFactory` with the platform work executor; the web application selects
the portable cooperative factory. Factory construction reports `PipelineError` when the selected
host cannot start supervision. Graph-runtime and processing code do not inspect the target.

Native and web application roots compose the UI `HostService` port. The browser adapter delegates
byte-oriented document selection, storage, and downloads to `platform`; the native
adapter delegates file access, configuration paths, and file/directory dialogs to
`NativeDocumentHost` while retaining product parsing and shell commands. Platform exposes the
repository mechanisms and allocates the application directory backing its native implementation;
the application roots select the repository and web fallback policy. Concrete source factories and
decoder scanners are retained per application instance: runtime behavior receives capability
overrides, while editor metadata receives stable-ID-keyed registration overrides through
`AppServices`. Node definitions and reusable crates read no process-global host configuration.
Cache identity, inspection, invalidation, cleanup, preview,
and producer-pruning policy remain in graph runtime and operate identically on the web OPFS-backed
repository and its memory fallback. The UI owns the portable configuration model, bundled fallback
fonts, and portable font installation. Native shell integrations, such as the macOS application
menu, publish portable commands and receive recent-document state through the app-owned host port.
Native composition installs the capture-export-owned repository-backed service; web composition
installs an explicit unavailable exporter.

The platform facade exposes individually scoped constructors for native paths, random-access file
bytes, output files, generic USB transfers, browser documents and downloads, repositories, and
worker mechanisms. Application roots combine those mechanisms with processing factories, concrete
device adapters, graph-worker protocols, fallbacks, and UI services. Platform does not select
nodes, formats, devices, graph behavior, or application policy.

## Isolated host adapter crate

Reusable core crates compile the same Rust source and module tree on native and web targets.
Matching public APIs backed by separate target-selected implementations inside a core crate are not
the final boundary. The platform adapter boundary covers host
services.

`platform` is the only reusable crate with general target selection and
target-specific dependencies. It is an adapter layer above the contract owners:

- `platform_artifacts` owns artifact-repository, prepared-source, and byte-region capability ports;
- `platform_runtime` owns host work and worker-operation capability ports and portable queue policy;
- `signal_runtime` owns stream execution, `signal_capture` owns finite-index capability ports, and
  `signal_capture_session` owns capture-session capability ports, and `signal_derived` owns
  derived-store capability ports;
- `logic_analyzer_capture_formats` owns concrete format behavior;
- `logic_analyzer_device_dslogic` owns DSLogic behavior and the transport port it consumes;
- `logic_analyzer_protocol_decoders`, `signal_transforms`, `signal_sinks`, and `signal_generators`
  own their respective portable processing behavior;
- `logic_analyzer_graph_runtime` owns cache-administration and source-preparation ports, including
  inline, capture-worker, and threaded source-preparation executors;
- `logic_analyzer_capture_export` owns export behavior and its application-facing service contract;
- `logic_analyzer_ui` owns dialog, host-command, cache-diagnostic, and document ports;
- `platform` supplies target-selected files, mmap, worker execution, browser
  handles, OPFS, downloads, and other host mechanisms;
- native and web application crates adapt and combine those mechanisms with domain services and
  inject the resulting application contracts.

The dependency never points from a core crate to `platform`. A capability port that
must be implemented by the adapter crate is a deliberate supported crate-root contract in its
behavioral owner. The owner exposes only platform-neutral request, result, capability, and error
types; implementation dependencies remain private to the adapter crate.

Portable implementations—including chunked memory storage, owned byte backing, deterministic
sources, discard sinks, and cooperative execution—remain in their behavioral owners and compile on
every target. Composition selects them explicitly. A web build does not obtain a synthetic source
or discard sink merely because a native capability is absent.

The only allowlisted reusable-crate exceptions are complete file-I/O compatibility constructors or
device-runtime leaves in `logic_analyzer_device_dslogic` that still require native execution, or
file-I/O compatibility leaves in `logic_analyzer_capture_formats`. Format
parsers and index factories consume prepared random-access sources; native application composition
acquires those sources through target-selected mechanisms. Node state, schemas, and builders remain
portable.

The processing-adapter allowlist is restricted to:

- `logic_analyzer_capture_formats::support::capture_archive::file_byte_source` and the DSL/Sigrok
  `path_compatibility` leaves that expose compatibility path constructors;
- the native U3Pro16 device-runtime leaves in `logic_analyzer_device_dslogic`, including its
  developer benchmark entry point.

DSL and Sigrok archive parsing, index construction, streaming, and prepared-source execution are
portable and compile on every target; only path acquisition remains in the compatibility leaves.

Sink implementations, graph builders, wasm synthetic/discard substitutes, and platform factory
selectors are not allowlisted. Decoder execution strategy, preferences, graph services, capture
export, cache administration, and viewer workers are also not processing exceptions; behavior and
domain adaptation remain in the owning domain crate or application composition root.

Architecture enforcement rejects target conditionals, target-selected module paths, target
inspection through `cfg!`, and target-specific dependencies outside `platform`, the
native/web bootstrap crates, and the explicit processing-adapter allowlist. It also verifies that
portable graph-node catalogs use one module tree on both targets and that core crates do not depend
on the adapter crate. `scripts/check_platform_boundaries.rb` is the machine-readable owner of this
allowlist. Its fixture tests and repository check run in CI before compilation; additions to the
allowlist therefore require an architecture-document update and an explicit checker change.

Application roots may depend directly on the UI facade, graph/node registration crates, domain
services, and low-level platform mechanisms needed for composition. The boundary checker rejects
core-to-platform dependencies, and the workspace manifest test rejects every platform dependency
on a Logic Conduit domain crate. Application modules select implementations and wire contracts but
do not implement reusable execution or data-plane policy.

The resolved dependency check restricts the DSLogic device crate to portable artifact, scheduling,
capture-session, capture, and typed-stream contracts. Its protocol consumes the crate-owned
`UsbTransport` and `DsLogicU3Pro16TransportFactory` ports. The native application implements those
ports with platform USB and file mechanisms, while deterministic device tests exercise the same
protocol through fake transports.

Target-conditioned synthetic sources and discard sinks are rejected even in otherwise allowlisted
target-selection locations; those portable implementations are chosen through explicit graph
configuration or injected capabilities.

## Enforcement

Workspace integration tests inspect Cargo's resolved dependency graph. They assert the exact local
dependency surfaces of generic capture, session, derived-data, runtime, viewer, node-editor,
trigger-editor, sink, capture-format, and DSLogic crates; prohibit domain and adapter edges from
generic compiler, graph, UI, widget, and platform owners; and include target-selected and
development edges where those are architectural constraints.

Compiled probes construct the real built-in and example-plugin inventories, verify registry and
capability descriptors, implement replaceable UI services with local fakes, and assert public port
type identity. Behavior tests exercise injected storage, capture, export, trigger-editing,
file-dialog, device-transport, and graph-service paths. Repository checks enforce module facades,
visibility, target-selection locations, and target-specific dependency allowlists.

Source-text assertions remain only for semantic constraints that Rust types and Cargo edges cannot
express, such as branching on persisted names or protocol labels, process-global state hidden
inside an allowed owner, prohibited vocabulary within a generic crate, and unrelated details in an
otherwise valid intra-crate port. Each retained assertion documents why it cannot be structural.
Workspace checks also run the compiler's `unreachable_pub` lint, and new warnings are treated as
visibility defects. Public API review includes re-exports, associated items, fields, variants,
native and wasm surfaces, and every type appearing in a public trait signature.
