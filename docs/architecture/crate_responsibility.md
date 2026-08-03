# Crate Responsibility Design

## Purpose

This document is the workspace-level map of crate responsibilities, dependency direction, and
the boundary changes that preserve one primary responsibility per crate. It complements the
cross-cutting aspect designs in `docs/aspects/`. A crate-specific design document defines the public contract,
invariants, extension points, owned persisted data, and permitted dependencies of its owner.

The terms *graph document*, *lowering*, and *execution* are distinct:

- a graph document is the editable node-editor model;
- lowering validates that document and produces a deterministic executable plan;
- execution materializes a plan, owns its lifecycle, and transports runtime observations.

Keeping those terms distinct prevents graph-document policy from accumulating in the processing
runtime or in the application UI.

## Responsibility map

| Crate | Primary responsibility | Must not own |
| --- | --- | --- |
| `signal_processing` | Generic processing runtime and generic capture, storage, indexing, derived-data, and acquisition contracts | Concrete sources, protocols, formats, widgets, graph documents, or target selection |
| `logic_analyzer_processing` | Concrete UI-independent sources, decoders, processing nodes, formats, devices, and sinks | Graph editor definitions, widget presentation, host selection, or application lifecycle |
| `logic_analyzer_graph_api` | Compile-time graph-node and payload-plugin contracts | Built-in nodes, compiler policy, UI state, or platform adapters; its current directory-catalog path contract is a documented exception to remove |
| `logic_analyzer_graph_nodes` | Built-in graph-node feature bundle: definitions, migrations, builders, payloads, and presentation metadata | Generic lowering, graph lifecycle, or target selection |
| `logic_analyzer_graph_compiler` | Graph-document semantic analysis, validation, lowering, inventory-derived catalog construction, execution, source preparation, cache policy, and worker protocol | Concrete node behavior, UI selection state, widgets, or target selection |
| `logic_analyzer_graph_runtime` (proposed) | Materialization, graph-run lifecycle, live reconciliation, source preparation, cache execution planning, and worker execution protocol | Editable graph UI state, concrete node behavior, or target selection |
| `logic_analyzer_viewer` | Generic waveform and derived-lane presentation | Node/protocol special cases, graph compilation, or source acquisition |
| `node_graph` | Generic graph document model, definitions, persistence reconciliation, and editor widget | Concrete node behavior, compiler policy, or host dialog implementation |
| `logic_analyzer_ui` | Application interaction and panel composition through explicit service ports | Concrete node definitions, target selection, processing execution policy, or host I/O |
| `logic_analyzer_platform` | Target-selected implementations of ports owned by the core crates, and their composition bundle | Core-domain policy or alternate core data models |
| `logic_analyzer_capture_export` | Streaming export of finalized generic capture storage | Graph concerns, concrete processing nodes, or UI policy |
| `logic_analyzer_test_support` | Cross-crate deterministic fixtures and contract-conformance helpers | Production composition and concrete UI behavior |
| Application crates | Thin native/web bootstrap and enabled-inventory composition | Reusable services, policy, storage, indexing, or execution |

The current source tree follows most of these directions. The principal mismatch is that
`logic_analyzer_graph_compiler` currently owns both lowering and graph execution through
`GraphCompiler`, `LiveRun`, source preparation, cache policy, and graph-worker types. This is a
real responsibility boundary, rather than a naming issue.

## Dependency direction

The target dependency graph is acyclic. An arrow means "depends on"; inventory consumption does
not create a dependency from generic compiler/runtime crates to a built-in node bundle.

```text
applications ──> platform ──> UI ──> graph runtime ──> graph compiler ──> graph API ──> node graph
                    │              │                    │                    │
                    │              ├────────────────────┴────────────────────┼──> signal processing
                    │              └─────────────────────────────────────────┴──> viewer
                    │
                    └──> built-in graph nodes ──> processing ──> signal processing
                                             ├──> graph API
                                             ├──> node graph
                                             └──> viewer ──> signal processing
```

`logic_analyzer_graph_runtime` depends on the compiler's public plan and catalog contract. The
compiler never depends on the runtime. A host can therefore validate and lower documents without
starting a processing graph, while every execution path consumes the same already-validated plan.

The built-in node bundle and third-party plugins submit inventory contracts without depending on
the compiler or runtime. `logic_analyzer_graph_runtime` reads those submissions only through the
graph API's validated catalog; there is no manifest dependency from runtime to a built-in bundle.
A plugin with optional UI presentation is split into a core feature crate and a UI companion crate;
only the companion depends on UI or viewer extension APIs.

## Proposed-future graph split

### Compiler

The compiler has one responsibility: transform a `node_graph::api::GraphState` plus an explicit
output-subscription plan into a deterministic `CompiledGraph` or semantic diagnostics. It owns:

- inventory validation and a read-only node catalog;
- graph traversal, pruning, socket/port and semantic-contract validation;
- kind negotiation, edge resolution, topological validation, and stable runtime identities;
- document-semantic discovery and edits that do not start work, such as node-owned configuration
  feature discovery;
- compiler diagnostics and the serializable execution-plan data types.

It does not retain an artifact repository, work executor, runtime-manager factory, active run,
source-preparation generation, or worker client. `GraphCompiler` is consequently replaced by a
stateless `GraphLowerer` or a small immutable `GraphCatalog` plus `GraphLowerer` facade. The name
`CompiledGraph` remains the execution-plan type; it does not imply that a run exists.

### Graph runtime

`logic_analyzer_graph_runtime` owns the operations that have an execution lifetime:

- materializing `CompiledGraph` nodes through the validated catalog;
- adding generated data collectors and configuring generic payload collection;
- cache lookup, cache maintenance scheduling, and persistent derived-lane preparation;
- finite source preparation and its progress state machine;
- `GraphRun`, run data, progress, diagnostics, stop, wait, and live graph reconciliation;
- source-process substitution for replay and live analysis;
- platform-neutral graph-worker request, response, and worker-runtime state machine.

It receives `AppManagerFactory`, `WorkExecutor`, `ArtifactRepository`, and worker transport ports
from composition. It does not inspect a target or directly create native threads, web workers,
paths, dialogs, USB transports, or browser objects. `signal_processing::AppManager` remains the
generic process-node executor; graph runtime only translates one compiled graph into its node and
connection specifications.

The UI's private service port composes a lowerer and a graph runtime. This preserves a small UI
test seam without making the UI downcast an arbitrary run to `LiveRun`. The platform adapter owns
the concrete worker transport and provides it through a graph-runtime port.

## Graph plugin contract boundaries

`RuntimeBuilder` currently combines graph semantics, runtime materialization, cache behavior,
source discovery, live capture, timeline editing, and presentation discovery. Its default methods
make new unrelated responsibilities cheap to add, so it is a high-risk extension point.

The graph API uses capability-specific contracts submitted together in one
`GraphNodeRegistration`:

| Contract | Consumer | Responsibility |
| --- | --- | --- |
| `GraphNodeSemantics` | Compiler | Port kinds, semantic connection contracts, requiredness, stable plan projection |
| `RuntimeMaterializer` | Graph runtime | Build a `ProcessNode`, runtime configuration, and restart classification |
| `CaptureSourceFeature` | Compiler and graph runtime | Capture identity, presentation description, and preparation factory |
| `LiveCaptureFeature` | UI service and graph runtime | Acquisition configuration and live-analysis source construction |
| `PresentationFeature` | UI | Renderer keys, lane/table descriptors, and panel metadata |
| `NodeMigration` | Graph-node feature at document load | Stable-ID state migration and user-visible warnings |

Optional capabilities remain explicit registration fields rather than methods that return an empty
default. The compiler can then depend only on semantics, the runtime only on materialization and
execution capabilities, and the UI only on presentation capabilities.

`DirectoryNodeCatalog` does not belong in `logic_analyzer_graph_api` in its current form because
its `PathBuf` configuration exposes a host filesystem concept from the plugin contract. The UI
owns the portable catalog presentation port and the platform owns directory discovery and
persistence. The cross-crate value is a catalog snapshot and diagnostic, never a host path.

## Generic processing decomposition

`signal_processing` is coherent at the highest level but its 18k-line implementation contains
five independently changing domains: stream execution, artifact storage, capture data/indexing,
derived-data storage, and capture-session control. A single root with broad re-exports hides those
ownership boundaries and makes unrelated dependencies appear equally fundamental.

The target is a small set of lower-level crates with an umbrella facade only if it preserves a
clear import path:

```text
signal_artifacts
    ├── signal_capture
    ├── signal_derived
    └── signal_capture_session

signal_runtime
    ├── signal_derived
    └── signal_capture_session
```

| Proposed owner | Responsibility | Current material |
| --- | --- | --- |
| `signal_artifacts` | Platform-neutral immutable byte regions, artifact identities, repository contracts, in-memory implementation, and replication | `storage` |
| `signal_runtime` | Typed ports and channels, `ProcessNode`, pipeline construction, schedulers, managers, generic work execution, and runtime errors | `node`, `ports`, `sender`, `receiver`, `pipeline`, `scheduler`, `manager`, `cooperative_manager`, `work_executor` |
| `signal_capture` | Generic capture source/index/query contracts and finite waveform indexing | `capture`, `waveform_index`, sample/edge query types |
| `signal_derived` | Generic payload registration, collected-lane contracts, sampling points, derived indexes, and encoded derived stores | `payload`, `derived_data_collector`, `derived_word_store`, `sampling_points`, `derived_index` |
| `signal_capture_session` | Driver-neutral acquisition lifecycle, session storage, capture policy, and trigger-program contracts | `live_capture`, `live_capture_store`, `capture_policy`, `advanced_trigger`, current `logic_analyzer` contracts |

This is a proposed future structure, not a mandate to create five crates immediately. The first
safe change is to establish these as private owner facades inside `signal_processing`, eliminate
cross-domain leaf imports, and give each facade a design document. Extraction occurs only after
the dependency direction is mechanically verified. `signal_artifacts` is the most valuable first
extraction because it is a lower-level capability used by capture and derived stores without
depending on either.

The umbrella name `signal_processing` remains suitable only if it exposes a curated compatibility
facade. New cross-crate imports use the owning domain path rather than a flat root re-export.
Compatibility re-exports are explicitly temporary and are removed through a versioned migration.

## Module rules

Every substantial owner module answers four questions in its module documentation:

1. What data and invariants does it own?
2. Which public or `pub(crate)` façade is its supported API?
3. Which adjacent owner modules may it depend on?
4. Which concerns are explicitly outside its boundary?

Leaf files implement one cohesive part of that owner. Large leaves are split by behavior, not by
arbitrary line count. `logic_analyzer_graph_compiler/src/graph.rs` currently contains lowering,
materialization, cache use, live diffing, runtime control, and tests; these become compiler
lowering modules and graph-runtime execution modules. Similar separation applies to the runtime
manager, capture stores, and UI coordinators when their facades expose more than one lifecycle.

Crate roots remain curated facades. A root re-exports only its primary public contract and the few
cross-domain value types necessary to use that contract. It does not become a second, flat module
system. The visibility and directory-backed-module rules in
[`Responsibility and Visibility Design`](../aspects/responsibility_visibility.md) remains the
normative visibility policy.

## Documentation set

The documentation set is organized by ownership and by cross-cutting aspect:

- `docs/architecture/` contains workspace-level composition designs: the responsibility map,
  graph composition, and application composition. These documents define relationships between
  owners; they do not replace an owner's own contract document.
- `docs/aspects/` contains only rules that intentionally span multiple owners: responsibility and
  visibility, native/web storage, live-capture and trigger control, plug-in payload/presentation,
  and testing.
- `docs/crates/<crate>.md` contains one present-tense design for every non-trivial crate. It
  includes responsibility, public façade, dependency allowlist, owned data and persistence,
  extension points, error boundary, and test boundary. Public API references live beside their
  crate owner when they are useful to embedders.
- `docs/modules/<crate>/<public-module>.md` is added when a public module or internal owner module
  contains an independent domain. It does not document every implementation leaf.
- `docs/integrations/` contains external protocol and decoder-host contracts. `docs/references/`
  contains hardware reference material.
- `docs/INDEX.md` is the entry point. It maps each crate and public module to exactly one owner
  design, and lists the cross-cutting aspects, API references, integrations, and references.

The documentation structure avoids duplicating behavior in several crate documents. An owner
document links to an aspect design for shared rules and records only how that owner satisfies the
rule. Proposed work remains in a clearly labeled proposed-future section or in `TODO.md`.

## Migration order

The following order preserves behavior and saved-graph compatibility while making each reviewable
change architectural.

1. Adopt this responsibility map, add `docs/INDEX.md`, and create owner documents for the graph
   compiler, proposed graph runtime, and `signal_processing` domains. Add architecture checks for
   the accepted dependency graph and for host paths outside platform/application code.
2. Split compiler lowering from execution in place, with the same public plan and diagnostics.
   Move `LiveRun`, execution cache policy, source preparation, worker runtime, and generated
   collectors to `logic_analyzer_graph_runtime`. Keep a short-lived compatibility façade only at
   the old crate root.
3. Replace the broad `RuntimeBuilder` with submitted capability contracts. Migrate every built-in
   node and plugin through explicit registrations. Node migration stays beside each concrete node
   and emits its existing user-visible warnings at the load boundary.
4. Move graph catalog directory configuration behind a UI-owned portable service and a
   platform-owned path adapter. Split UI-capable plugins into core and presentation companions.
5. Establish private `signal_processing` owner facades, then extract `signal_artifacts` and
   `signal_runtime` after their dependency tests pass. Extract capture and derived domains only
   when consumers need independent release or compilation boundaries.

No migration changes stable node IDs, payload IDs, serialized graph state, graph extensions, or
renderer keys. Saved-document compatibility is implemented by the affected concrete graph-node or
UI document migration and reported to the user; generic compiler and viewer code does not infer
legacy behavior from names.

## Architectural acceptance criteria

- Lowering a document neither allocates runtime storage nor starts source preparation or a graph.
- Starting a graph consumes a `CompiledGraph` and cannot perform document-semantic rewrites.
- `logic_analyzer_graph_compiler` has no `AppManager`, `ProcessNode`, repository, executor,
  worker-client, or active-run field in production code.
- `logic_analyzer_graph_runtime` has no widget, `egui`, concrete node/protocol, path, or target
  dependency.
- `logic_analyzer_graph_api` contains no filesystem path, dialog, or target-specific contract.
- A core graph/runtime plugin depends only on graph API and lower-level runtime contracts; optional
  UI/viewer behavior lives in a companion crate.
- Artifact, runtime, capture, derived, and capture-session owners have only documented downward
  dependencies and no flat root imports for new code.
- Every public module and every independently-owned internal domain has one linked design document
  and one façade path.
