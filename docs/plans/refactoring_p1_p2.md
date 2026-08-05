# P1/P2 Refactoring Directions

This is a working plan, not an architecture document: it describes intended changes in future
tense and is consumed together with [`TODO.md`](../../TODO.md), which owns the item list,
priorities, and ordering constraints. Delete each section here when its item completes and the
resulting architecture is documented in `docs/architecture/` or `docs/aspects/`.

Line numbers below were correct when this plan was written; verify them before editing, and trust
the named function/type over the number when they disagree.

## Ground rules for the implementer

- Read `AGENTS.md` first. The module-facade rules (declarations only in `lib.rs`/`mod.rs`,
  directory-backed public modules, explicit re-exports) are mandatory and reviewed.
- One TODO item — often one numbered step of one item — per branch/PR. Do not combine items.
- These are relocations and rewirings, not redesigns. Public behavior, saved-graph compatibility,
  stable IDs, and output fingerprints must not change. If a step seems to require a behavior
  change, stop and flag it instead of improvising.
- After each step: `cargo test -p <every touched crate>`, then the workspace integration package
  (`cargo test -p logic-analyzer-examples`), then clippy. The web build must keep compiling:
  `cargo check -p logic-analyzer-app-web --target wasm32-unknown-unknown` (see CI config for the
  exact invocation).
- The existing source-text architecture tests (`architecture_tests.rs`, string `contains` checks)
  will break when code moves. Update the string to match the new reality — or, where a moved rule
  now has a structural check from the [tests item](#tests-architecture-structural), delete the
  string test in the same PR.

## composition.application-roots (P1) {#composition-application-roots}

**Problem.** `logic_analyzer_platform` assembles the whole application.
`standard_services()` (`crates/logic_analyzer_platform/src/platform/native.rs:114`, and the web
variant at `platform/web.rs:25`) constructs `AppServices`, selects concrete nodes by name, and
applies the capability-override list (`binary_file_writer`, `csv_word_writer`,
`text_file_writer`, `dsl_file_source`, `sigrok_file_source`, `sigrok_decoder`, `u3pro16`). The
app crates (`crates/app_native/src/native.rs:139` and `app_web`) only call it. This is why
platform's manifest depends on `logic-analyzer-graph-nodes` — an adapter crate sitting above the
application it serves.

**Target.** `app_native` and `app_web` are the composition roots. Platform exposes narrow public
constructors for each adapter it owns; the apps call `logic_analyzer_graph_nodes::
*_capability_override(...)` themselves, passing platform adapters in, and build `AppServices`.

**What platform keeps.** Adapters implementing contracts defined by core crates:
`NativeArtifactRepository`, `NativeWorkExecutor`, `NativeWorkerOperationExecutor`,
`NativeOutputStorage`, the U3Pro16 transport factory, the Sigrok Python runtime and directory
scanner, dialogs/menus/fonts. The traits these implement live in `logic_analyzer_processing`,
`signal_artifacts`, `signal_runtime`, and (today, wrongly) `logic_analyzer_graph_nodes` — see the
dependency note below. A platform→processing manifest edge is therefore legitimate and remains;
the platform→graph-nodes and platform→ui edges are the ones being removed.

**Steps.**

1. Promote the adapter constructors `standard_services()` uses to `pub` platform API, each
   returning the contract type (`Arc<dyn DslFileSourceFactory>`, `Arc<dyn SigrokDecoderRuntime>`,
   `Arc<dyn WorkExecutor>`, …). Most already exist as private `native_*` functions in `native.rs`
   — promote and document them; do not rewrite them.
2. Move the body of `standard_services()` — override-vec assembly plus the `AppServices` builder
   chain — essentially verbatim into `app_native::native` (and the web equivalent into
   `app_web`). The apps already depend on `logic-analyzer-graph-nodes` and `logic-analyzer-ui`,
   so no new app dependencies are needed.
3. Delete `standard_services()` and shrink or delete `PlatformServices`
   (`crates/logic_analyzer_platform/src/services.rs`) — once apps assemble `AppServices`, the
   bundle holds nothing platform-specific. Its accessors (`artifact_repository()`,
   `work_executor()`, …) become individual platform constructors the app calls.
4. Remove `logic-analyzer-graph-nodes` from platform's `Cargo.toml`. **This will fail until the
   [host-factory-injection step](#composition-host-factory-injection) moves the
   `SigrokDecoderRuntime`/`SigrokCatalogScanner` trait definitions out of
   `logic_analyzer_graph_nodes::host_configuration` — platform implements those traits.** Do that
   trait move first (it is step 1 of the other item) or land the two items together.
5. The `install_sigrok_catalog_scanner` / `install_file_source_factories` calls currently inside
   `standard_services()` (native.rs:128–133) move with the body to the apps as an interim state;
   they disappear entirely when host-factory-injection completes.

**Acceptance.** Platform's `Cargo.toml` lists neither `logic-analyzer-graph-nodes` nor (after the
[inversion item](#composition-platform-ui-inversion)) `logic-analyzer-ui`. Adding a new device or
format touches processing (behavior), graph-nodes (node definition), and the app crates (wiring)
— not platform, unless a new host adapter is genuinely needed.

## composition.platform-ui-inversion (P1) {#composition-platform-ui-inversion}

**Problem.** Platform implements port traits that `logic_analyzer_ui` defines, so the
platform→ui manifest edge survives application-roots on its own. The implementations:
`NativeHostService` (`platform/native.rs`), the web `HostService` (`platform/web_document.rs`),
`NodeCatalogService` (`platform/native_sigrok/catalog.rs`), `CaptureExportService`
(`platform/native_capture_export.rs`). Platform also consumes UI-owned value types:
`ApplicationSettings`, `HostCommand`, `HostUiCapabilities`, `OpenDialog`, `SaveDialog`,
`ModifierKeyLabels`, `DecodedBlockCacheSnapshot`, `NodeCatalogSnapshot`, `APPLICATION_ID`,
`default_input_bindings` (see the imports at `platform/native.rs:38`).

**Chosen direction.** Extract the host-port contracts into a new small crate below both
consumers — working name `logic-analyzer-host-ports` (`crates/logic_analyzer_host_ports`). This
beats moving the adapter implementations into the app crates because the implementations are
large, shared between both apps, and genuinely platform code; only the *contract location* is
wrong.

**Steps.**

1. Inventory exactly what platform imports from `logic_analyzer_ui` (grep `logic_analyzer_ui::`
   under `crates/logic_analyzer_platform/src`). That import list *is* the port surface.
2. Verify the port types are UI-framework-free. If any trait method or value type mentions `egui`
   types, split that method out or replace the type with a neutral one before moving — the ports
   crate must not depend on `egui`.
3. Create the ports crate owning those traits and value types. Move them; do not copy.
4. `logic_analyzer_ui` depends on the ports crate and **re-exports every moved symbol from its
   existing facade paths**, so the apps and other consumers compile unchanged. UI-internal code
   keeps using its own facade.
5. Point platform's imports at the ports crate and delete `logic-analyzer-ui` from platform's
   `Cargo.toml`.
6. Judgment calls, resolved as follows: `APPLICATION_ID` and `default_input_bindings` move to the
   ports crate (they are product/host constants both sides need). `ApplicationSettings` moves if
   platform only loads/persists it; if UI mutates it richly, split the persisted record (ports)
   from UI behavior. `DecodedBlockCacheSnapshot` likely belongs to `signal_derived` — check where
   it is produced before moving it to ports.

**Acceptance.** Platform's manifest has no `logic-analyzer-ui`; ports crate has no `egui`,
UI, widget, or platform dependency; apps compile without import changes.

## composition.host-factory-injection (P2) {#composition-host-factory-injection}

**Problem.** `crates/logic_analyzer_graph_nodes/src/host_configuration.rs` holds process-global
slots: `SIGROK_CATALOG_SCANNER` (`OnceLock`, line 58), `DSL_FILE_SOURCE_FACTORY` and
`SIGROK_FILE_SOURCE_FACTORY` (`OnceLock<RwLock<…>>`, lines 59–61), installed from platform
(`native.rs:128–133`, `web.rs:94`). Initialization order is significant, two application
instances in one process are impossible, and tests with different hosts cannot run concurrently.

**Key facts.**

- The traits `SigrokDecoderRuntime` and `SigrokCatalogScanner` (host_configuration.rs:23–48) are
  typed entirely in `logic_analyzer_processing` and `signal_runtime` types. They belong next to
  `logic_analyzer_processing::nodes::decoders::sigrok_decoder`. Moving them is what lets
  application-roots drop the platform→graph-nodes edge.
- The Sigrok *decoder runtime* is already injected correctly: `sigrok_decoder_capability_override
  (runtime)` carries the `Arc` into the builder (`nodes/decoders/sigrok_decoder/builder.rs:71`).
  That is the pattern to replicate.
- Only one code path reads a global outside the install functions:
  `nodes/decoders/sigrok_decoder/definition.rs:564` calls `sigrok_catalog_scanner().scan(…)`.
  Find who calls that function — it will be node-template/definition construction — and thread
  the scanner (or a pre-scanned `SigrokCatalogSnapshot`) through that call chain instead. Note
  `sigrok_node_templates(snapshot)` (host_configuration.rs:152) already takes a snapshot
  argument; converging on snapshot-passing is the likely shape.
- Grep for remaining readers of `dsl_file_source_factory()` / `sigrok_file_source_factory()`
  (the `pub(crate)` getters, lines 83–89). Each reader is a place where a builder or definition
  needs the factory carried through its capability override or registration input, the same way
  the decoder runtime is carried.

**Steps.**

1. Move the two trait definitions into `logic_analyzer_processing` (public module
   `nodes::decoders::sigrok_decoder`); leave `pub use` re-exports in `logic_analyzer_graph_nodes`
   temporarily so callers migrate incrementally, and remove them at the end.
2. For each global reader found above, thread the dependency explicitly (constructor argument,
   capability-override payload, or registration input). No default-to-unavailable global fallback
   — the *builder* may still default to an unavailable backend, as `SigrokDecoderBuilder::default`
   does today.
3. Delete the statics and `install_*` functions; delete their call sites in platform/apps.
4. Add a test that constructs two registries/app-service bundles with different fakes in one
   process and shows they do not observe each other.

**Acceptance.** No `OnceLock`/`RwLock`/`static` host state in `logic_analyzer_graph_nodes`;
`install_` no longer appears in platform or app crates.

## graph.document-model-extraction (P2) {#graph-document-model-extraction}

**Problem.** `logic_analyzer_graph_plan`, `graph_runtime`, and `graph_capabilities` import only
`node_graph::api::{NodeId, Socket}` (e.g. `graph_plan/src/plan/types.rs:11`,
`graph_capabilities/src/node/contracts.rs:5`), yet the manifest edge pulls the whole egui widget
crate into the execution tier and web workers.

**Constraint discovered in code.** The `model` leaf files use egui types: `model/node.rs` uses
`egui::{Color32, Pos2}`, `model/socket.rs` and `model/graph.rs` use `Color32`. So the *full*
document model cannot move without an egui decision. Do not make that decision here.

**Scope: minimal first slice only.** Extract the identity types the execution tier needs —
`NodeId`, `SocketId`, `Socket`, `SocketDirection` (all in `node_graph/src/model/ids.rs` and
`model/socket.rs`) — into a new crate, working name `node-graph-document`
(`crates/widgets/node_graph_document`).

1. Check whether `Socket` itself carries a `Color32` field. If it does not (color is likely on
   socket *definitions*, not the identity), move it as-is. If it does, move only the identity
   types and introduce nothing new — instead check whether `graph_capabilities` can take the
   fields it actually reads; flag for review rather than inventing a parallel `SocketRef` type.
2. Serialization is a persisted contract: `NodeId`/`SocketId` appear in saved graphs. The move
   must be serde-transparent. Add a round-trip test that deserializes one of the checked-in
   `graphs/*.json` examples before and after and compares.
3. `node_graph` depends on the new crate and re-exports the moved types from both its crate root
   and `api`, exactly where they are exported today (`node_graph/src/api/mod.rs:109` re-exports
   `crate::model::{…}`). Widget-crate consumers compile unchanged.
4. Switch imports in `graph_plan`, `graph_runtime`, `graph_capabilities`, `graph_orchestration`;
   remove `node-graph` from those four manifests. `graph_compiler` still needs `GraphState` and
   keeps its `node-graph` dependency for now — shrinking that is part of the P5 standalone-crate
   item, not this one.
5. The new crate's manifest: `serde` only. No egui, no widget-support, no input-bindings.

**Acceptance.** `grep node-graph crates/logic_analyzer_graph_{plan,runtime,capabilities,
orchestration}/Cargo.toml` finds nothing; saved-graph round-trip test passes; a structural
manifest check (next item) locks the edge.

## tests.architecture-structural (P2) {#tests-architecture-structural}

**Problem.** ~1,670 lines of `architecture_tests.rs` across the workspace `include_str!` sibling
files and assert `.contains("…")` (largest: `graph_nodes` 319 lines, `graph_compiler` 287,
`processing` 185). They break on renames, pass when the string appears in a comment, and prove
nothing about the compiled contract.

**Direction.**

1. Add one workspace-level test in the top-level integration package
   (`logic-analyzer-examples`, which owns cross-crate tests per the testing strategy) that runs
   `cargo metadata --format-version 1` via `std::process::Command`, parses it with the already
   available `serde_json`, and asserts the *forbidden edge list*:
   - `logic-analyzer-platform` ↛ `logic-analyzer-ui`, ↛ `logic-analyzer-graph-nodes`
     (activates as the composition items land — until then mark the assertion `#[ignore]` with
     the TODO item ID in the ignore reason);
   - `logic-analyzer-graph-{plan,runtime,capabilities,orchestration}` ↛ `node-graph`
     (after the extraction item);
   - `logic-analyzer-graph-compiler` ↔ `logic-analyzer-graph-runtime`: neither depends on the
     other; runtime also ↛ registry;
   - `logic-analyzer-graph-nodes` and `example-plugin` ↛ compiler;
   - `logic-analyzer-ui` ↛ `logic-analyzer-processing`, ↛ `logic-analyzer-graph-nodes`.
   Assert on the dependency *graph* (resolve `id`/`dependencies` from metadata), not on raw
   `Cargo.toml` text, so target-specific and dev-dependencies are handled deliberately: dev-deps
   are allowed unless the rule says otherwise.
2. Capability rules become registry-construction tests: build a `GraphRegistry` snapshot from the
   real inventories (the compiler tests already consume the public immutable registry — follow
   that pattern) and assert on the resulting descriptors: every registration with a semantics has
   a materializer, override stable-IDs resolve, duplicate IDs rejected, and so on. Most of these
   assertions already exist as registry unit tests — the work is deleting the string tests that
   duplicate them, not writing new ones.
3. Go through each `architecture_tests.rs` rule by rule: delete rules now covered structurally;
   keep a string test only where no structural probe exists (e.g. "no `std::env` access in
   tests"), and add a one-line comment saying why it stays textual.
4. Do not chase 100% conversion in one PR. Priority order: the manifest-edge test (it guards the
   other P1/P2 items), then `graph_nodes`/`graph_compiler` (the two largest files), then the
   rest opportunistically.

## signal.tier-naming (P2 · decision) {#signal-tier-naming}

This item is a *decision to record*, not a refactoring to execute. The recommendation already in
TODO.md: name the tier for this application and let it use domain vocabulary directly — no second
domain consumes it, and `DigitalLaneSnapshot`, `TriggerLaneSnapshot`, `ProtocolPacket`,
`SimpleTriggerCondition`, and the `logic_analyzer` module already live there.

**To execute the decision:** add a short "Tier vocabulary" section to
`docs/architecture/vocabulary_and_concepts.md` stating: (1) the `signal_*` crates are
application-tier infrastructure for this product, not a domain-neutral framework; (2) domain
vocabulary is therefore permitted in them, and the placement rule for a new type is *lowest crate
whose stated responsibility covers it*, never "keep it generic just in case"; (3) crate renames
are expressly out of scope — the names stay until a rename is worth its churn on its own merits.
Then update the affected TODO items ([session.domain-relocation],
[derived.payload.builtin-registration]) to reference the recorded decision instead of the open
question. If the maintainer instead chooses the separation option, those two items become purges
and this section must be rewritten — do not guess; the maintainer makes this call explicitly.

**Ordering.** Record the decision before starting [processing.domain-split] so new crate names
and type placements follow one rule.

## processing.domain-split (P2) {#processing-domain-split}

**Problem.** `logic_analyzer_processing` is 28k lines defined by a negation ("concrete"). Current
contents: capture formats and archive support (`src/support/{capture_archive,capture_format,
dsl_file,sigrok_file,capture_index.rs}`), a USB device (`nodes/sources/dslogic_u3pro16`), five
decoders (`nodes/decoders/{i2c,spi,uart,parallel,sigrok}_decoder` plus `support/sigrokdecode`),
twelve logic primitives (`nodes/logic/*`), sinks (`nodes/sinks/*`), synthetic sources, and five
binaries under `src/bin/` with `clap`/`tracing-subscriber` as *library* dependencies.

**Gate.** Record the [tier-naming decision](#signal-tier-naming) first.

**Target crates** (names follow the recorded tier decision; these assume the domain-vocabulary
option):

| New crate | Takes | Positive responsibility |
| --- | --- | --- |
| `logic-analyzer-capture-formats` | `support/capture_archive`, `capture_format`, `dsl_file`, `sigrok_file`, `capture_index.rs`, plus `nodes/sources/{dsl_file,sigrok_file}` | Reading and indexing DSL and Sigrok capture files |
| `logic-analyzer-device-dslogic` | `nodes/sources/dslogic_u3pro16` and its protocol/transport support | DSLogic U3Pro16 acquisition |
| `logic-analyzer-protocol-decoders` | `nodes/decoders/*`, `support/sigrokdecode` | Protocol decoding, including the Sigrok decoder host contract |
| `logic-analyzer-processing` (residual, may rename) | `nodes/logic/*`, `nodes/sinks/*`, synthetic sources, `types` | Generic logic primitives and sinks |

**Steps, in PR-sized units.**

1. Move the five `src/bin/` binaries into the top-level `logic-analyzer-examples` package (which
   already owns benches and workspace tooling); drop `clap` and `tracing-subscriber` from the
   library manifest. Cheapest step, no consumer impact — do it first.
2. Extract `logic-analyzer-capture-formats`. It is the lowest layer (device and decoders may
   depend on archive/format contracts, never the reverse).
3. Extract `logic-analyzer-device-dslogic`, then `logic-analyzer-protocol-decoders`.
4. Update consumers per step — `logic_analyzer_graph_nodes` is the main importer
   (`logic_analyzer_processing::nodes::…` paths), plus platform's factory implementations and
   the test-support crate. Do **not** leave long-lived re-export shims in the residual crate;
   update the imports in the same PR so the split is real in the manifests.
5. Per the testing strategy, each moved module's `test_data/` moves with it; fixtures stay with
   the owning crate.

**Invariants.** Node stable IDs, saved state, definition names, output fingerprints, and the
public trait contracts (`DslFileSourceFactory`, `DsLogicU3Pro16SourceFactory`,
`SigrokDecoderRuntime` after the injection item moves it here) are relocations only. Rustdoc
facade docs move with their modules. `docs/architecture/crate_responsibility.md` and the
dependency diagrams gain the new crates in the same PR that creates each one.
