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
| `signal_artifacts` | Platform-neutral immutable byte regions, artifact identities, repository contracts, in-memory storage, replication, and shared persistence time/integrity primitives | Capture formats, derived encodings, stream execution, host adapters, or target selection |
| `signal_runtime` | Generic typed-stream execution, scheduling, process-node lifecycle, and work dispatch | Signal payload definitions, capture/index contracts, storage, sessions, concrete nodes, graph documents, or target selection |
| `signal_capture` | Generic immutable capture payloads, source/index/query contracts, random-access signal capabilities, and finite waveform indexing | Acquisition sessions, derived data, concrete formats or devices, graph documents, UI, or target selection |
| `signal_derived` | Generic derived payloads, retained-lane collection, sampling points, indexes, and encoded derived storage | Capture sessions, concrete protocols or nodes, viewers, graph documents, UI, or target selection |
| `signal_capture_session` | Generic acquisition lifecycle, capture-session storage, capture policy, and trigger-program contracts | Stream execution, immutable capture/index ownership, derived-data ownership, artifact repository ownership, concrete sources, protocols, formats, widgets, graph documents, or target selection |
| `logic_analyzer_processing` | Concrete UI-independent sources, decoders, processing nodes, formats, devices, and sinks | Graph editor definitions, widget presentation, host selection, or application lifecycle |
| `logic_analyzer_graph_capabilities` | Graph-node and payload capability contracts | Inventory assembly, built-in nodes, compiler policy, UI state, or platform adapters |
| `logic_analyzer_graph_registry` | Graph-node and payload registration descriptors, inventory collection, validation, host overrides, and immutable catalog snapshots | Graph documents, lowering, generated collectors, execution lifetimes, UI state, or target selection |
| `logic_analyzer_graph_nodes` | Built-in graph-node feature bundle: definitions, migrations, builders, payloads, and presentation metadata | Generic lowering, graph lifecycle, or target selection |
| `logic_analyzer_graph_plan` | Neutral immutable processing-graph, payload-materialization, subscription, and sampling contracts | Graph documents, lowering, registry access, execution lifetimes, UI state, or target selection |
| `logic_analyzer_graph_compiler` | Graph-document semantic analysis, validation, lowering, and discovery | Runtime services, execution lifetimes, concrete node behavior, UI selection state, widgets, or target selection |
| `logic_analyzer_graph_runtime` | Processing-graph materialization, graph-run lifecycle, live reconciliation, source preparation, and cache execution planning | Editable graph documents, compiler or registry services, concrete node behavior, or target selection |
| `logic_analyzer_graph_orchestration` | Application-neutral worker protocol and composition of compiler and runtime services | Graph semantics, processing-plan contracts, concrete nodes, UI state, or target selection |
| `logic_analyzer_viewer` | Generic waveform and derived-lane presentation | Node/protocol special cases, graph compilation, or source acquisition |
| `node_graph` | Generic graph document model, definitions, persistence reconciliation, and editor widget | Concrete node behavior, compiler policy, or host dialog implementation |
| `logic_analyzer_ui` | Application interaction and panel composition through explicit service ports | Concrete node definitions, target selection, processing execution policy, or host I/O |
| `logic_analyzer_platform` | Target-selected implementations of ports owned by the core crates, and their composition bundle | Core-domain policy or alternate core data models |
| `logic_analyzer_capture_export` | Streaming export of finalized generic capture storage | Graph concerns, concrete processing nodes, or UI policy |
| `logic_analyzer_test_support` | Cross-crate deterministic fixtures and contract-conformance helpers | Production composition and concrete UI behavior |
| Application crates | Thin native/web bootstrap and enabled-inventory composition | Reusable services, policy, storage, indexing, or execution |

The graph boundary separates document semantics from work with an execution lifetime. Compiler
calls are safe for validation and discovery-only hosts; runtime calls consume their immutable
plans and injected execution services.

## Dependency direction

The target dependency graph is acyclic. An arrow means "depends on"; inventory consumption does
not create a dependency from generic compiler/runtime crates to a built-in node bundle.

```text
applications ──> platform ──> UI ──> graph compiler ──> graph registry ──> graph capabilities
                    │              │          └────────> graph plan ──────────────┤
                    │              ├──────────────> graph runtime ──> graph plan ─┤
                    │              └────────> graph orchestration ──> compiler + runtime
                    │                                                       │
                    └──> built-in graph nodes ──> processing ───────────────┴──> signal capture session
                    │                                      ├──────────────────> signal capture
                    │                                      ├──────────────────> signal derived
                    │                                      └──────────────────> signal runtime
                    │
                    └──> built-in graph nodes ──> processing ──> signal capture session
                                             ├──> graph registry ──> graph capabilities
                                             ├──> node graph
                                             └──> viewer ──> signal capture + signal derived
```

`logic_analyzer_graph_compiler` and `logic_analyzer_graph_runtime` both depend on the neutral
`logic_analyzer_graph_plan` contract and do not depend on each other. The compiler embeds resolved
materializer handles and payload behavior while lowering; the runtime consumes that completed
plan. `logic_analyzer_graph_orchestration` sits above both only for worker-hosted composition.

The built-in node bundle and third-party plugins submit registry-owned descriptors containing
graph-API capabilities without depending on the compiler or runtime. The compiler and UI read
those submissions through `logic_analyzer_graph_registry`; there is no manifest dependency
from those generic consumers to a built-in bundle.
A plugin with optional UI presentation is split into a core feature crate and a UI companion crate;
only the companion depends on UI or viewer extension APIs.

## Graph compiler and runtime split

### Compiler

The compiler has one responsibility: transform a `node_graph::api::GraphState` plus an explicit
output-subscription plan into a deterministic neutral `ProcessingGraph` or semantic diagnostics. It owns:

- read-only access to graph and payload capabilities through a registry snapshot;
- graph traversal, pruning, socket/port and semantic-contract validation;
- kind negotiation, edge resolution, topological validation, and stable runtime identities;
- document-semantic discovery and edits that do not start work, such as node-owned configuration
  feature discovery;
- compiler diagnostics.

It does not retain an artifact repository, runtime-manager factory, active run, or
source-preparation generation. `GraphLowerer` is the stateless facade over an immutable
`GraphRegistry`.

### Graph runtime

`logic_analyzer_graph_runtime` owns the operations that have an execution lifetime:

- materializing `ProcessingGraph` nodes through compiler-resolved materializer handles;
- adding generated data collectors and configuring generic payload collection;
- cache lookup, cache maintenance scheduling, and persistent derived-lane preparation;
- finite source preparation and its progress state machine;
- `GraphRun`, run data, progress, diagnostics, stop, wait, and live graph reconciliation;
- source-process substitution for replay and live analysis;

It receives `AppManagerFactory`, `WorkExecutor`, and `ArtifactRepository` from composition. It does
not inspect a target or directly create native threads, web workers,
paths, dialogs, USB transports, or browser objects. `signal_runtime::AppManager` remains the
generic process-node executor; graph runtime only translates one compiled graph into its node and
connection specifications.

The UI's private service adapter owns a lowerer and a separate graph runtime. It lowers the current
document before Run or apply and passes the resulting `ProcessingGraph` into the runtime. This preserves a small UI
test seam without making the UI downcast an arbitrary run to `LiveRun`. The platform adapter owns
the concrete worker transport and provides it through the UI graph-service port. The neutral
graph-orchestration crate owns the worker message, codec, client, and worker-side compiler/runtime
composition required by that adapter.

## Graph plugin contract boundaries

Graph-node registration separates graph semantics, runtime materialization, cache behavior, source
discovery, live capture, timeline editing, and presentation discovery. Each consumer receives only
the capability it owns, and optional features are explicit rather than default aggregate methods.

The graph registry groups graph-API capability contracts into one `GraphNodeRegistration`:

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

`NodeCatalogService` is a UI-owned portable port. Its snapshots contain stable namespaces,
host-formatted directory labels, scan status, diagnostics, and completed node templates. Platform
adapters own directory selection, filesystem paths, persistence, and scanning; host paths never
enter graph capability contracts or UI state.

## Generic processing decomposition

The generic data plane is divided into explicit owners. `signal_artifacts` owns platform-neutral
artifacts and byte sources, `signal_runtime` owns stream execution, `signal_capture` owns immutable
capture and finite indexing, `signal_derived` owns retained derived data, and
`signal_capture_session` owns capture-session control. Consumers import each contract from its
owner; there is no umbrella facade.

Their dependency direction is:

```text
signal_artifacts
    ├── signal_capture
    ├── signal_derived
    └── signal_capture_session

signal_runtime
    ├── signal_derived
    └── signal_capture_session
```

Runtime payload negotiation uses generic type, stream-semantics, capacity, and capability
contracts. Architecture tests enforce the one-way dependencies and prevent any owner from
redirecting another owner's symbols through its crate root.

## Module rules

Every substantial owner module answers four questions in its module documentation:

1. What data and invariants does it own?
2. Which public or `pub(crate)` façade is its supported API?
3. Which adjacent owner modules may it depend on?
4. Which concerns are explicitly outside its boundary?

Leaf files implement one cohesive part of that owner. Large leaves are split by behavior, not by
arbitrary line count. Compiler graph leaves contain document semantics and lowering, while graph
runtime leaves contain materialization, cache use, live diffing, and runtime control. Similar
separation applies to the runtime manager, capture stores, and UI coordinators when their facades
expose more than one lifecycle.

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
- A public module or independently-owned internal domain documents its contract in Rustdoc at its
  owning source facade. It does not document every implementation leaf.
- `docs/integrations/` contains external protocol and decoder-host contracts. `docs/references/`
  contains hardware reference material.
- `docs/INDEX.md` is the entry point. It maps each crate and public module to exactly one owner
  design, and lists the cross-cutting aspects, API references, integrations, and references.

The documentation structure avoids duplicating behavior in several crate documents. An owner
document links to an aspect design for shared rules and records only how that owner satisfies the
rule. Proposed work remains in a clearly labeled proposed-future section or in `TODO.md`.

## Architectural acceptance criteria

- Lowering a document neither allocates runtime storage nor starts source preparation or a graph.
- Starting a graph consumes a `ProcessingGraph` and cannot perform document-semantic rewrites.
- `logic_analyzer_graph_compiler` has no graph-runtime dependency, repository, executor, or
  active-run field.
- `logic_analyzer_graph_runtime` has no compiler, registry, editable graph document, widget,
  `egui`, concrete node/protocol, path, or target dependency.
- `logic_analyzer_graph_capabilities` contains no inventory assembly, filesystem path, dialog, or
  target-specific contract.
- `logic_analyzer_graph_registry` contains no graph documents, lowering, execution lifecycle, UI,
  concrete node, or target dependency.
- A core graph plugin depends only on graph capabilities, graph registry, and lower-level runtime contracts;
  optional UI/viewer behavior lives in a companion crate, and the graph runtime remains registry
  independent.
- Artifact, runtime, capture, derived, and capture-session owners have only documented downward
  dependencies and no flat root imports for new code.
- Every public module and every independently-owned internal domain has one linked design document
  and one façade path.
