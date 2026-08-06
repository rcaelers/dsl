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
   - `platform` ↛ `logic-analyzer-ui`, ↛ `logic-analyzer-graph-nodes`
     (activates as the composition items land — until then mark the assertion `#[ignore]` with
     the TODO item ID in the ignore reason);
   - `logic-analyzer-graph-{plan,runtime,capabilities,orchestration}` ↛ `node-graph`
     (after the extraction item);
   - `logic-analyzer-graph-compiler` ↔ `logic-analyzer-graph-runtime`: neither depends on the
     other; runtime also ↛ registry;
   - `logic-analyzer-graph-nodes` and `example-plugin` ↛ compiler;
   - `logic-analyzer-ui` ↛ `logic-analyzer-{capture-formats,device-dslogic,protocol-decoders}`,
     ↛ `signal-{generators,sinks,transforms}`, ↛ `logic-analyzer-graph-nodes`.
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
