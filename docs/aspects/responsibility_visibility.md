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
- `signal_capture` owns immutable generic capture, query, and finite indexing contracts.
- `signal_derived` owns generic derived-data payload, collection, indexing, and storage contracts.
- `signal_capture_session` owns generic capture-session
  contracts. It consumes fixed-width byte ranges, stable source identities, prepared
  random-access sources, immutable byte regions, and portable memory sources directly from
  `platform_artifacts` without re-exporting them. Host paths, files, mappings, and browser handles
  are absent from those contracts. Its public capture
  vocabulary is `Capture*`; it does not expose DSL, Sigrok, USB, decoder, graph-node, or UI
  terminology.
- `logic_analyzer_processing` owns concrete capture formats, devices, protocol decoders,
  processing nodes, and sinks. Format parsing and device-transport errors originate here and are
  mapped to generic runtime errors only where a generic trait requires it.
- `logic_analyzer_graph_capabilities` owns graph-node and payload capability contracts.
- `logic_analyzer_graph_registry` owns graph-node and payload registration, inventory validation,
  and immutable catalog assembly.
- `logic_analyzer_graph_nodes` owns built-in concrete node definitions, builders, migrations,
  registrations, and presentation metadata.
- `logic_analyzer_graph_compiler` owns generic graph lowering, document discovery, and semantic
  diagnostics.
  Definition defaults and lowering helpers remain crate-private unless plugin authors or another
  crate implement against a documented contract.
- `logic_analyzer_graph_plan` owns the immutable processing-plan contract exchanged between plan
  producers and consumers.
- `logic_analyzer_graph_runtime` owns materialization, source preparation, cache planning,
  execution lifecycle, collected run data, and live reconciliation.
- `logic_analyzer_graph_orchestration` owns the graph-worker protocol and worker-side composition
  above a separate compiler and runtime.
- `node_graph::api` owns the compiler-facing graph document and node-definition contracts.
  Compiler and graph-node code depend on this namespace; widget and editor operations remain at
  the `node_graph` crate root for UI composition. File controls depend on its portable
  `FileDialogService`; the widget defaults to an unavailable implementation and the application
  composition injects the host adapter.
- `logic_analyzer_capture_export` owns native streaming export of finalized generic capture
  storage plus the stateful export-service contract and asynchronous native implementation. It
  depends on capture contracts and format libraries, not graph crates, UI, platform, or concrete
  processing nodes.
- `logic_analyzer_test_support` owns deterministic capture providers and data-plane conformance
  fixtures shared by cross-crate tests. It depends on generic runtime contracts rather than
  concrete processing, graph, or UI crates.
- `logic_analyzer_ui` owns the application-facing graph service port. Application orchestration
  depends on its private `GraphService` and `GraphRun` traits; the crate's production
  adapter composes `GraphLowerer`, `GraphRuntime`, and `LiveRun`, while UI tests provide deterministic local
  implementations. Its public `HostService` port owns file and directory dialogs, graph-document
  persistence, derived-cache commands and diagnostics, and native-shell state exchange. Native and
  web application roots implement that application-facing port by adapting low-level host
  mechanisms. The UI consumes and re-exports the `logic_analyzer_capture_export` service contract;
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
The native application platform owns the application namespace and operating-system directory
policy, then passes resolved paths through configuration. Generic crates do not inspect host
environment variables to choose an application location.

## Visibility rules

Use the narrowest visibility that contains every intended consumer:

- private for implementation details used in one module;
- `pub(super)` for collaboration with the direct parent or sibling modules through that parent;
- `pub(crate)` for an internal crate contract;
- `pub` only for a supported cross-crate or plugin contract.

A `pub` item hidden below a private module is still an invalid declaration unless its wider
visibility is required by a public signature. Public re-exports are deliberate API decisions,
not a convenience for internal imports.

Public traits expose a complete implementable contract. Every type in their required method
signatures is publicly nameable from a stable path. Conversely, implementation seams that are
not supported extension points remain private, including their generic parameters and errors.

## Module layout

The workspace uses an owner-facade module layout.

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
| `signal_derived` | `derived_word_store` | The public module owns the independently usable encoded annotation-store contract; other payload, lane, sampling, and index contracts are exposed through the crate facade. |
| `signal_capture_session` | `live_capture`, `live_capture_store`, `logic_analyzer` | These are substantial generic capture-session domains. `live_capture` owns the provider-neutral configured and prepared acquisition contracts. `logic_analyzer` owns the driver-neutral capture, trigger, and processing-source contracts consumed by concrete device nodes. Lower-level runtime, capture, and derived contracts are imported directly from their owning crates and are not re-exported. |
| `logic_analyzer_processing` | `nodes`, `nodes::decoders`, `nodes::logic`, `nodes::sinks`, `nodes::sources`, each node module under its family, `types` | Each concrete node owns a directory-backed public facade, so its configuration, factory, and discovery contracts have an unambiguous owner such as `nodes::decoders::parallel_decoder::StrobeMode` or `nodes::decoders::sigrok_decoder::SigrokDecoderDescriptor`. The crate root exposes the shared `ProcessNodeConstruction` factory result and lazy capture-source metadata contracts. Shared implementation support is crate-private. Protocol-neutral processing value conventions are exposed through `types`. Node implementation, transport, and format details remain private behind their owning node facade. |
| `logic_analyzer_graph_capabilities` | `node`, `node_support` | `node` owns capability traits implemented by graph-node plugins. `node_support` owns open port identity, protocol-neutral presentation descriptions, capture descriptions, decoder-table contracts, and the restricted node build context. It contains no graph-node or payload inventory assembly, compiler, host, built-in-node, UI, or export operations. |
| `logic_analyzer_graph_registry` | none | Its crate root exposes graph-node, payload, and protocol-presentation registration descriptors, validated inventory access, and the immutable `GraphRegistry`. Implementation modules remain private. |
| `logic_analyzer_graph_plan` | none | Its crate root exposes the immutable `ProcessingGraph`, processing-node/edge, payload-materialization, subscription, sampling, and diagnostic contracts exchanged between compiler and runtime. |
| `logic_analyzer_graph_compiler` | none | Its crate root exposes `GraphLowerer` and document-discovery results. Processing-plan values are imported from `logic_analyzer_graph_plan`, capability contracts from `logic_analyzer_graph_capabilities`, and registry contracts from `logic_analyzer_graph_registry`; the compiler crate does not forward them. |
| `logic_analyzer_graph_runtime` | none | Its crate root exposes `GraphRuntime`, `LiveRun`, run-data and source-preparation results, cache operations, and source-preparation execution contracts. Execution implementation modules remain private. |
| `logic_analyzer_graph_orchestration` | none | Its crate root exposes graph-worker request, message, codec, client, and worker-runtime contracts. Lowering and execution behavior remain in their owning crates. |
| `logic_analyzer_graph_nodes` | none | The crate root exposes the linker anchor plus host-injection and portable-template helpers for concrete node contracts. Built-in graph-node definitions, socket types, migrations, presentations, inventory submissions, and crate-local test fixtures remain private. Cross-component fixtures belong to the top-level integration-test package. |
| `logic_analyzer_capture_export` | none | The cohesive exporter exposes its curated format, progress, observer, report, stateful service contract, and native asynchronous implementation through the crate root. Encoder, archive, and service implementation modules remain private. |
| `logic_analyzer_platform` | none | The crate root exposes individually scoped target-selected host constructors. Private native and web modules contain the single reusable target-selection point. |
| `logic_analyzer_test_support` | none | Shared deterministic acquisition providers and data-plane conformance fixtures are exposed through the crate root. Their synchronization, repository observation, and fixture implementations remain private. |
| `node_graph` | `api` | `api` exposes graph documents, identifiers, sockets, and node-definition contracts to compilers and graph-node implementations. The crate root exposes the widget/editor composition surface used by UI hosts. |
| `logic_analyzer_viewer` | none | The reusable viewer exposes one curated crate-root API; drawing, sampling, input, cursor, lane, worker, and indexing modules remain private. |
| `logic_analyzer_ui` | none | The application-composition crate exposes only its host-facing crate-root facade, including portable host service contracts such as `NodeCatalogService`. |
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

## Platform surfaces

Native and wasm reusable crates share the platform-neutral data model and compile the same source
tree. Native-only filesystem, USB, mmap, worker, export, and host-integration capabilities are
selected as complete adapter modules in `logic_analyzer_platform`. A platform facade exposes a
complete contract; consumers do not depend on an unnameable backend type or a target-dependent
collection of incidental helpers.

`AppManager` owns one portable facade over an injected `AppManagerBackend`. The native application
combines `PipelineAppManagerFactory` with the platform work executor; the web application selects
the portable cooperative factory. Graph-runtime and processing code do not inspect the target.

Native and web application roots compose the UI `HostService` port. The browser adapter delegates
byte-oriented document selection, storage, and downloads to `logic_analyzer_platform`; the native
adapter delegates file access, configuration paths, and file/directory dialogs to
`NativeDocumentHost` while retaining product parsing and shell commands. Platform exposes the
repository mechanisms and allocates the application directory backing its native implementation;
the application roots select the repository and web fallback policy.
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

`logic_analyzer_platform` is the only reusable crate with general target selection and
target-specific dependencies. It is an adapter layer above the contract owners:

- `platform_artifacts` owns artifact-repository, prepared-source, and byte-region capability ports;
- `platform_runtime` owns host work and worker-operation capability ports and portable queue policy;
- `signal_runtime` owns stream execution, `signal_capture` owns finite-index capability ports, and
  `signal_capture_session` owns capture-session capability ports, and `signal_derived` owns
  derived-store capability ports;
- `logic_analyzer_processing` owns concrete format and device behavior and the transport ports that
  behavior consumes;
- `logic_analyzer_graph_runtime` owns cache-administration and source-preparation ports, including
  inline, capture-worker, and threaded source-preparation executors;
- `logic_analyzer_capture_export` owns export behavior and its application-facing service contract;
- `logic_analyzer_ui` owns dialog, host-command, cache-diagnostic, and document ports;
- `logic_analyzer_platform` supplies target-selected files, mmap, worker execution, browser
  handles, OPFS, downloads, and other host mechanisms;
- native and web application crates adapt and combine those mechanisms with domain services and
  inject the resulting application contracts.

The dependency never points from a core crate to `logic_analyzer_platform`. A capability port that
must be implemented by the adapter crate is a deliberate supported crate-root contract in its
behavioral owner. The owner exposes only platform-neutral request, result, capability, and error
types; implementation dependencies remain private to the adapter crate.

Portable implementations—including chunked memory storage, owned byte backing, deterministic
sources, discard sinks, and cooperative execution—remain in their behavioral owners and compile on
every target. Composition selects them explicitly. A web build does not obtain a synthetic source
or discard sink merely because a native capability is absent.

The only allowlisted reusable-crate exceptions are complete file-I/O compatibility constructors or
device-runtime leaves in `logic_analyzer_processing` that still require native execution. Format
parsers and index factories consume prepared random-access sources; native application composition
acquires those sources through target-selected mechanisms. Node state, schemas, and builders remain
portable.

The processing-adapter allowlist is restricted to:

- `support::capture_archive::file_byte_source` and the DSL/Sigrok
  `path_compatibility` leaves that expose compatibility path constructors;
- the native U3Pro16 device-runtime leaves under `nodes::sources::dslogic_u3pro16`, including its
  developer benchmark entry point.

DSL and Sigrok archive parsing, index construction, streaming, and prepared-source execution are
portable and compile on every target; only path acquisition remains in the compatibility leaves.

Sink implementations, graph builders, wasm synthetic/discard substitutes, and platform factory
selectors are not allowlisted. Decoder execution strategy, preferences, graph services, capture
export, cache administration, and viewer workers are also not processing exceptions; behavior and
domain adaptation remain in the owning domain crate or application composition root.

Architecture enforcement rejects target conditionals, target-selected module paths, target
inspection through `cfg!`, and target-specific dependencies outside `logic_analyzer_platform`, the
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
Target-conditioned synthetic
sources and discard sinks are rejected even in otherwise allowlisted target-selection locations;
those portable implementations are chosen through explicit graph configuration or injected
capabilities.

## Enforcement

Architecture tests protect prohibited dependency and terminology directions. Workspace checks
run the compiler's `unreachable_pub` lint, and new warnings are treated as visibility defects.
Public API review includes re-exports, associated items, fields, variants, native and wasm
surfaces, and every type appearing in a public trait signature.
