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
- Retained source-text architecture assertions protect only semantic constraints that dependency
  metadata and compiled probes cannot express. When code moves, preserve the constraint and its
  explanatory comment; delete the textual assertion if the rule becomes structurally observable.
