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

## tests.architecture-structural (P2) {#tests-architecture-structural}

**Problem.** ~685 lines of `architecture_tests.rs` remain across the workspace that `include_str!`
sibling files and assert `.contains("…")` (largest: `logic_analyzer_viewer` 66 lines, followed by
`signal_runtime` and the UI live-capture suite at 59 each). They break on renames, pass when the string
appears in a comment, and prove nothing about the compiled contract.

The top-level integration package parses `cargo metadata --format-version 1` and asserts the
resolved non-dev dependency graph. Its forbidden-edge contract is:

1. The workspace dependency graph rejects:
   - `platform` ↛ `logic-analyzer-ui`, ↛ `logic-analyzer-graph-nodes`
     and platform has no other workspace dependencies than its two neutral contract owners;
   - `logic-analyzer-graph-{plan,runtime,capabilities,orchestration}` ↛ `node-graph`;
   - `logic-analyzer-graph-compiler` ↔ `logic-analyzer-graph-runtime`: neither depends on the
     other; runtime also ↛ registry;
   - `logic-analyzer-graph-nodes` and `example-plugin` ↛ compiler;
   - `logic-analyzer-ui` ↛ `logic-analyzer-{capture-formats,device-dslogic,protocol-decoders}`,
     ↛ `signal-{generators,sinks,transforms}`, ↛ `logic-analyzer-graph-nodes`.
   - `logic-analyzer-viewer` depends only on generic input, artifact, capture, session, and
     derived-data contracts within the workspace.
   - `signal-derived` depends only on generic artifact, execution, and capture contracts within the
     workspace.
   - `signal-runtime` depends only on the neutral host-scheduling contract within the workspace.
   Target-specific edges participate in the resolved graph; dev-dependencies are allowed.
2. The real built-in and example-plugin inventories construct a `GraphRegistry` snapshot in a
   cross-crate test. Registration descriptors must match the snapshot, override stable IDs resolve,
   and duplicate overrides are rejected.

**Direction.**

1. Go through each remaining `architecture_tests.rs` rule by rule: delete rules now covered structurally;
   keep a string test only where no structural probe exists (e.g. "no `std::env` access in
   tests"), and add a one-line comment saying why it stays textual.
2. Prioritize the UI service suites. Do not replace an implementation-text check with another
   filename-sensitive source scan; prefer a dependency edge, public API probe, registry construction,
   or behavior test.
