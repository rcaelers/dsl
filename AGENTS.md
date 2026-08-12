# Architecture boundaries

- Keep `node_graph`, `logic_analyzer_viewer`, and generic compiler/runtime
  infrastructure independent of concrete nodes and protocols. They must not
  branch on node names, port labels, or protocol-specific values (for example
  UART, `Bits`, `Data`, start/stop markers, SPI, or Binary Decoder).
- Concrete behavior belongs in the corresponding `logic_analyzer_graph_nodes` node feature and
  its capture-format, device, protocol-decoder, transform, sink, or generator runtime owner.
- Pass protocol-specific presentation needs to generic infrastructure through
  explicit, generic metadata/contracts. Do not infer behavior from display
  names or use name-based special cases.
- Preserve saved-graph compatibility through explicit node migration/load
  handling with user-visible warnings; do not hide compatibility work in
  generic viewer/compiler code.

See `docs/aspects/plugin_extensible_payload.md` for the detailed payload and viewer-lane decision.

# Crate boundaries

- Place every type, function, and implementation module under the component whose
  stated responsibility includes that behavior. Do not expose unrelated helpers from
  a domain module or crate merely because their implementation is reusable. When
  multiple domains need a capability, extract it into a neutral lower-level module or
  crate with that capability as an explicit responsibility; consumers depend on that
  shared owner rather than reaching through one another. Treat `pub`, `pub(crate)`,
  re-exports, and module visibility as architectural contracts, not convenience access.
- `platform_artifacts` owns platform-neutral byte-source, artifact, repository, replication, and
  persistence contracts.
- `platform_runtime` owns platform-neutral host-work, worker-operation, kernel, and bounded-queue
  contracts.
- `signal_runtime` owns generic typed-stream execution, scheduling, and pipeline supervision.
- `signal_capture` owns immutable generic capture, query, and finite-index contracts.
- `signal_derived` owns generic derived-data payload, collection, query, indexing, and storage
  contracts.
- `signal_capture_session` owns generic acquisition, recording, capture-source lifecycle, and
  driver-neutral logic-analyzer contracts.
- `logic_analyzer_capture_formats` owns UI-independent DSL and Sigrok capture readers, indexes,
  and replay sources.
- `logic_analyzer_device_dslogic` owns DSLogic acquisition behavior and its neutral transport port.
- `logic_analyzer_protocol_decoders` owns UI-independent concrete protocol decoders.
- `signal_transforms`, `signal_sinks`, and `signal_generators` own portable transforms, terminal
  consumers, and deterministic sources respectively.
- `logic_analyzer_graph_capabilities` owns the contracts and neutral values implemented by graph
  features and payloads.
- `logic_analyzer_graph_registry` owns headless graph-feature and payload inventory validation and
  immutable capability snapshots.
- `logic_analyzer_graph_nodes` owns concrete graph nodes and their builders.
- `node_graph_document` owns portable persisted graph records and semantic socket identities.
- `logic_analyzer_graph_editor_registry` owns the product integration between stable graph-feature
  IDs and `node_graph` editor definitions; headless graph crates do not depend on it.
- `logic_analyzer_graph_plan` owns the immutable processing-plan contract exchanged between
  lowering and execution.
- `logic_analyzer_graph_compiler` owns generic graph-document discovery, semantic validation,
  capability negotiation, diagnostics, and lowering.
- `logic_analyzer_graph_runtime` owns source preparation, cache planning, materialization,
  execution lifecycle, run data, and live reconciliation.
- `logic_analyzer_graph_orchestration` owns the application-neutral graph-worker protocol and
  worker-side composition of the separate compiler and runtime.
- `logic_analyzer_capture_export` owns finalized-capture export behavior and its application-facing
  service contract.
- `platform` owns reusable native and web host adapters. It implements the neutral contracts owned
  by `platform_artifacts` and `platform_runtime` and is the only reusable crate that selects code
  or dependencies by compilation target.
- `logic_analyzer_ui` composes the widgets and application services; it must not
  contain concrete node definitions or runtime builders.
- Native and web application crates are thin composition roots. They bootstrap their host, obtain
  generic `platform` mechanisms, adapt application and domain ports, and inject the resulting
  services; they do not own storage, indexing, caching, processing, or execution policy.
- Reusable widgets live below `crates/widgets` and must remain independent of
  concrete nodes and protocols.

See `docs/aspects/responsibility_visibility.md` for symbol ownership,
visibility, error-boundary, and enforcement rules.

# Module layout and facades

The owner-facade layout below is mandatory throughout the Rust workspace.

1. Module declarations occur only in `lib.rs`, `main.rs`, and `mod.rs`. Test modules are the only
   exception: they may occur in any Rust file, but their module names must contain `tests`.
2. Modules are private by default. Symbols needed by another module are selectively re-exported
   by the owning `mod.rs` or crate `lib.rs`; consumers import the facade path rather than a leaf
   implementation path.
3. Within one module, leaf files import symbols from sibling leaf modules directly (for example,
   `super::presentation::render` or `super::definition::State`). They do not consume symbols
   re-exported by their own `mod.rs`; those re-exports exist only for consumers outside the module.
4. Public modules are limited API namespaces. The public-module allowlist is maintained in
   `docs/aspects/responsibility_visibility.md`; every module absent from it is private. Adding
   a public module requires an explicit design update and API review.
5. Every public module is directory-backed and has a `mod.rs`. Do not create a public module
   backed directly by a sibling `.rs` file.
6. A `mod.rs` contains only module documentation, attributes on declarations or re-exports,
   module declarations, and re-exports. Put structs, enums, traits, implementations, functions,
   constants, type aliases, executable macro bodies, and other implementation code in leaf files.
7. Use private visibility for details confined to one leaf file. Use `pub(crate)` for symbols
   shared directly between sibling leaf modules or re-exported through an internal crate facade;
   sibling-only symbols are not re-exported by `mod.rs`. Use `pub` only for supported cross-crate
   or plugin contracts re-exported through an allowed public facade. Do not use `pub(super)` or
   `pub(in ...)`.
8. Struct fields are private by default. Behavioral or invariant-owning structs expose methods.
   Plain record types intended for construction or pattern matching may expose fields, but all
   data fields use one visibility matching the record contract; do not mix private, `pub(crate)`,
   and `pub` data fields in one struct.

See the module layout and public-module allowlist in
`docs/aspects/responsibility_visibility.md`.

# Platform boundaries

- All reusable runtime, compiler, viewer, graph, widget, UI, and portable processing crates compile
  the same Rust source on native and wasm. They do not contain target-selected modules, target
  conditionals, target-dependent public surfaces, or target-specific dependencies.
- The neutral lower-level crates own platform-neutral capability traits and algorithms. `platform`
  implements their native and web host adapters. Application roots adapt and inject those
  mechanisms into application and domain contracts. `platform` contains the single reusable
  target-selection point.
- Native and web application crates may contain only target-specific entry, bootstrap, and
  application-level adaptation required to inject `platform` mechanisms into owner contracts.
- Complete file-I/O leaves in `logic_analyzer_capture_formats` and native device-runtime leaves in
  `logic_analyzer_device_dslogic` are the only
  permitted reusable-crate exception when the capability cannot yet be injected without moving
  concrete format or device behavior to the platform crate. Every exception is explicitly
  allowlisted in `docs/aspects/responsibility_visibility.md`. Node state, schemas, builders,
  protocol state machines, and portable processing behavior remain identical on every target.
- Synthetic sources, discard sinks, in-memory repositories, and cooperative executors are explicit
  portable implementations selected through configuration or injection. They are not implicit
  wasm substitutes for native behavior.
- New target-specific code outside `platform`, the application bootstrap crates, or
  an explicitly allowlisted processing adapter is prohibited. Existing splits are migration work
  tracked in `TODO.md`, not precedent for adding another split.

See `docs/aspects/native_web_storage.md` for the unified native/web data-plane,
host-adapter, source-parity, and exception design.

# Change discipline

- Scope one `TODO.md` item — often one numbered step of one item — per branch. Do not combine
  items.
- Refactorings are relocations and rewirings, not redesigns: public behavior, saved-graph
  compatibility, stable IDs, and output fingerprints do not change. A step that seems to require
  a behavior change is flagged and decided explicitly, not improvised.
- After each change: `cargo test -p <crate>` for every touched crate, then the workspace
  integration package (`cargo test -p logic-analyzer-examples`), then clippy. Keep the web build
  compiling:
  `cargo check -p logic-analyzer-app-web --target wasm32-unknown-unknown --all-targets --all-features`.
- Source-text architecture assertions exist only for semantic constraints that dependency
  metadata and compiled probes cannot express. When code moves, preserve the constraint and its
  explanatory comment; delete the textual assertion once the rule is structurally observable in
  `tests/architecture_dependencies_tests.rs` or a compiled registry check.

# Design documentation

- Design documents describe the current architecture in present tense.
- Treat unqualified design statements as implemented system behavior; do not
  add implementation-status labels, completed rollout steps, resolved-problem
  sections, or implementation history.
- Put unimplemented ideas only in clearly labeled proposed-future sections and
  track actionable work in `TODO.md`.
- Use version control for historical context instead of preserving it in
  current design documents.

# Rust imports

- Group `use` statements in this order, separated by one blank line:
  language crates (`std`, `core`, `alloc`), third-party crates, other crates
  in this workspace, then the current crate (`crate`, `self`, `super`).
- Run `scripts/sort_use_groups.rb` after adding or reorganizing imports;
  ordinary `cargo fmt` preserves the workspace-specific split.
