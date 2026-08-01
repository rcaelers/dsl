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

- `signal_processing` owns generic runtime, capture, storage, indexing, and derived-data
  contracts. Its public capture vocabulary is `Capture*`; it does not expose DSL, Sigrok, USB,
  decoder, graph-node, or UI terminology.
- `logic_analyzer_processing` owns concrete capture formats, devices, protocol decoders,
  processing nodes, and sinks. Format parsing and device-transport errors originate here and are
  mapped to generic runtime errors only where a generic trait requires it.
- `logic_analyzer_graph_api` owns graph-node and payload plugin contracts.
- `logic_analyzer_graph_nodes` owns built-in concrete node definitions, builders, migrations,
  registrations, and presentation metadata.
- `logic_analyzer_graph_compiler` owns generic graph lowering, discovery, execution, and host services.
  Definition defaults and lowering helpers remain crate-private unless plugin authors or another
  crate implement against a documented contract.
- `node_graph::api` owns the compiler-facing graph document and node-definition contracts.
  Compiler and graph-node code depend on this namespace; widget and editor operations remain at
  the `node_graph` crate root for UI composition.
- `logic_analyzer_capture_export` owns native streaming export of finalized generic capture
  storage. It depends on capture contracts and format libraries, not graph crates or concrete
  processing nodes.
- `logic_analyzer_test_support` owns deterministic capture providers shared by cross-crate tests.
  It depends on generic runtime contracts rather than concrete processing, graph, or UI crates.
- `logic_analyzer_ui` owns the application-facing graph service port. Application and platform
  orchestration depend on its private `GraphService` and `GraphRun` traits; the crate's production
  adapter delegates to `GraphCompiler` and `LiveRun`, while UI tests provide deterministic local
  implementations. Its private `HostService` port owns native file and directory dialogs, graph
  document persistence, and derived-cache commands. Native and web adapters implement the same
  platform-neutral contract in complete platform-selected modules. Its private
  `CaptureExportService` port owns asynchronous export startup, progress, cancellation, and
  completion; `CaptureCoordinator` supplies only a finalized session identity and retains capture
  lifecycle policy. Native dialog and filesystem-export adapters are enabled only by the native
  application host; the default UI crate build uses unavailable adapters so its component tests do
  not link those concrete backends.
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
- Target selection uses attributes on complete module declarations and re-exports in an allowed
  root file. It does not require inline implementation modules or executable selection macros in
  a `mod.rs`.

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
| `signal_processing` | `capture`, `live_capture`, `live_capture_store`, `logic_analyzer`, `derived_word_store`; native-only `waveform_index` | These are substantial, independent generic capture and storage domains. `live_capture` owns the provider-neutral configured and prepared acquisition contracts. `logic_analyzer` owns the driver-neutral capture, trigger, and processing-source contracts consumed by concrete device nodes. Runtime plumbing such as ports, senders, receivers, scheduling, workers, errors, and pipeline implementation remains private behind root re-exports. |
| `logic_analyzer_processing` | `nodes`, `nodes::decoders`, `nodes::logic`, `nodes::sinks`, `nodes::sources`, each node module under its family, `types` | Each concrete node owns a directory-backed public facade, so its configuration, factory, and discovery contracts have an unambiguous owner such as `nodes::decoders::parallel_decoder::StrobeMode` or `nodes::decoders::sigrok_decoder::SigrokDecoderDescriptor`. The crate root exposes the shared `ProcessNodeConstruction` factory result and lazy capture-source metadata contracts. Shared implementation support is crate-private. Protocol-neutral processing value conventions are exposed through `types`. Node implementation, transport, and format details remain private behind their owning node facade. |
| `logic_analyzer_graph_api` | `node`, `node_support` | `node` owns the traits and inventory submissions implemented by graph-node plugins. `node_support` owns open port identity, protocol-neutral presentation descriptions, capture descriptions, decoder-table contracts, and the restricted node build context. It contains no compiler, host, built-in-node, UI, or export operations. |
| `logic_analyzer_graph_compiler` | none | Its crate root exposes `GraphCompiler`, host result types, `CompileCtx` result extraction, and saved-document operations consumed by application hosts. Graph-node and node-support contracts are imported from `logic_analyzer_graph_api`; the compiler crate does not forward them. |
| `logic_analyzer_graph_nodes` | none | The crate root exposes only the linker anchor and native catalog discovery. Built-in graph-node definitions, socket types, migrations, presentations, inventory submissions, and crate-local test fixtures remain private. Cross-component fixtures belong to the top-level integration-test package. |
| `logic_analyzer_capture_export` | none | The cohesive native exporter exposes its curated format, progress, observer, report, and export operation through the crate root. Encoder and archive implementation modules remain private. |
| `logic_analyzer_platform` | none | The crate root exposes its opaque composition bundle and constructors. Private target-selected modules implement host capabilities owned by the core contract crates. |
| `logic_analyzer_test_support` | none | Shared deterministic acquisition providers are exposed through the crate root. Their synchronization and acquisition implementations remain private. |
| `node_graph` | `api` | `api` exposes graph documents, identifiers, sockets, and node-definition contracts to compilers and graph-node implementations. The crate root exposes the widget/editor composition surface used by UI hosts. |
| `logic_analyzer_viewer` | none | The reusable viewer exposes one curated crate-root API; drawing, sampling, input, cursor, lane, worker, and indexing modules remain private. |
| `logic_analyzer_ui` | none | The application-composition crate exposes only its host-facing crate-root facade. |
| `input_bindings`, `panel_layout`, `trigger_editor`, `widget_support` | none | Each crate already represents one cohesive public component and does not need a second namespace level. |
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
The existing `-D unreachable-pub` check remains enabled.

## Error boundaries

Generic errors describe failures at the abstraction boundary, such as I/O, invalid generic
indices, or malformed generic storage. Concrete parsers and transports own their detailed error
types and dependencies. When they implement a generic source or runtime trait, they translate a
concrete error into a generic boundary error without moving the concrete dependency into the
generic crate.

## Current platform surfaces

Native and wasm public surfaces share the platform-neutral data model. Native-only filesystem,
USB, mmap, worker, export, and host-integration capabilities are selected as complete modules or
registry entries. A platform facade exposes a complete contract; consumers do not depend on an
unnameable backend type or a target-dependent collection of incidental helpers.

`AppManager` is one such facade. Its public type and operations are identical on every target;
whole implementation files delegate to the threaded native manager or cooperative wasm manager.

`logic_analyzer_platform` composes the UI `HostService` port today. It selects complete native and
web adapter modules and returns an opaque service bundle to the application bootstrap. The native
adapter owns dialogs, graph-document file I/O, persistent-cache administration, and the
derived-cache and live-capture-session directories. It also owns native configuration-file discovery
and I/O; the UI owns the portable configuration model. It supplies optional system symbol fonts while
the UI owns bundled fallback fonts and portable installation. Native shell integrations, such as the
macOS recent-document menu, receive portable UI state through the same host-service contract. The web
adapter reports unavailable storage capabilities and supplies embedded configuration. The UI does not
select either implementation.

## Proposed future: isolated host adapter crate

Reusable core crates compile the same Rust source and module tree on native and web targets.
Matching public APIs backed by separate target-selected implementations inside a core crate are not
the final boundary. The existing platform adapter boundary expands to cover the remaining host
services.

`logic_analyzer_platform` is the only reusable crate with general target selection and
target-specific dependencies. It is an adapter layer above the contract owners:

- `signal_processing` owns storage, byte-region, execution, and capture capability ports;
- `logic_analyzer_processing` owns concrete format and device behavior and the transport ports that
  behavior consumes;
- `logic_analyzer_graph_compiler` owns cache-administration and source-preparation ports;
- `logic_analyzer_ui` owns dialog, host-command, and export-orchestration ports;
- `logic_analyzer_platform` depends on those crates and implements their ports with files, mmap,
  native workers, browser handles, OPFS, native dialogs, export destinations, and USB transports;
- native and web application crates construct those adapters and inject them through thin
  composition roots.

The dependency never points from a core crate to `logic_analyzer_platform`. A capability port that
must be implemented by the adapter crate is a deliberate supported crate-root contract in its
behavioral owner. The owner exposes only platform-neutral request, result, capability, and error
types; implementation dependencies remain private to the adapter crate.

Portable implementations—including chunked memory storage, owned byte backing, deterministic
sources, discard sinks, and cooperative execution—remain in their behavioral owners and compile on
every target. Composition selects them explicitly. A web build does not obtain a synthetic source
or discard sink merely because a native capability is absent.

The only temporary reusable-crate exceptions are complete file-I/O or USB adapter leaf modules in
`logic_analyzer_processing` for which extraction would otherwise move concrete format or device
behavior into the platform crate. An exception is explicitly allowlisted, contains only host
access, and excludes node state, schemas, builders, parsers, protocol state machines, and portable
runtime behavior. The intended exception allowlist is empty after source/destination and USB
transport injection is complete.

The temporary processing-adapter allowlist is restricted to the host-access leaves of:

- `nodes::sources::dsl_file::platform`;
- `nodes::sources::sigrok_file::platform`;
- `nodes::sinks::binary_file_writer::platform`;
- `nodes::sinks::csv_word_writer::platform`;
- `nodes::sinks::text_file_writer::platform`;
- `nodes::sources::dslogic_u3pro16::platform`, limited to USB discovery and transport.

The enclosing node modules, their builders, parsers, encoders, device state machines, and wasm
synthetic/discard substitutes are not allowlisted. Decoder execution strategy, embedded-runtime
hosting, preferences, graph services, capture export, cache administration, and viewer workers are
also not processing exceptions; their adapters move to `logic_analyzer_platform` while their
behavior remains in the owning core crate.

Architecture enforcement rejects target conditionals, target-selected module paths, target
inspection through `cfg!`, and target-specific dependencies outside `logic_analyzer_platform`, the
native/web bootstrap crates, and the explicit processing-adapter allowlist. It also verifies that
portable graph-node catalogs use one module tree on both targets and that core crates do not depend
on the adapter crate.

## Enforcement

Architecture tests protect prohibited dependency and terminology directions. Workspace checks
run the compiler's `unreachable_pub` lint, and new warnings are treated as visibility defects.
Public API review includes re-exports, associated items, fields, variants, native and wasm
surfaces, and every type appearing in a public trait signature.
