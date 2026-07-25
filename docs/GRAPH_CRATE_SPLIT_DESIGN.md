# Graph Crate Responsibility Split

## Current boundary

`logic_analyzer_graph_api` owns the compile-time plugin contract through its `node` and
`node_support` namespaces. Graph-node and collected-payload inventory types are defined there;
inventory assembly is compiler-owned and consumes submissions without importing a node bundle.

`logic_analyzer_graph_nodes` owns the built-in node definitions, builders, migrations, socket
types, payload presentations, and inventory submissions. `logic_analyzer_graph_compiler` owns compiler and
application-facing services. Its feature-gated `GraphCompiler` test constructors are the narrow integration seam
for isolated built-in-node tests.
`logic_analyzer_capture_export` owns native streaming capture export without depending on a graph
crate. `logic_analyzer_test_support` owns deterministic capture providers shared by cross-crate
tests. Native and web application composition link the built-in bundle before constructing the host
compiler.

## Architecture

The graph domain is divided into crates whose dependency edges follow the direction of their
contracts:

```text
node_graph                 signal_processing
    ^                            ^
    |                            |
    +---- logic_analyzer_graph_api ---- logic_analyzer_viewer
                    ^
          +---------+----------+
          |                    |
logic_analyzer_graph_compiler   logic_analyzer_graph_nodes
          ^                    ^
          |                    |
 logic_analyzer_ui          plugins

logic_analyzer_capture_export ---> signal_processing
logic_analyzer_test_support  ---> signal_processing
```

`logic_analyzer_graph_compiler` and `logic_analyzer_graph_nodes` both depend on
`logic_analyzer_graph_api`. The compiler crate never depends on the built-in-node crate. A plugin
depends on the API crate and the lower-level domains required by its own implementation; it does
not depend on the compiler or application UI.

### Graph API crate

`logic_analyzer_graph_api` is the supported compile-time extension contract. It has two public,
directory-backed namespaces and no application-host operations.

`node` contains contracts implemented or submitted by a graph-node feature:

- `RuntimeBuilder`;
- `GraphNodeRegistration`;
- `CollectedPayloadRegistration`;
- `LiveCaptureFeature`;
- `CaptureGraphSourceFactory`.

`node_support` contains data and restricted services supplied to those implementations:

- `PortKind`, `PortValue`, `ResolvedInput`, and `ResolvedInputs`;
- `NodeBuildContext`;
- state decoding at the node-owned error boundary;
- capture identity and presentation descriptions;
- default waveform and decoder-table presentation descriptions, resolved table sources, and their
  registry;
- sampling overlay and qualifier descriptions;
- trigger configuration, simple-trigger channels, and live-capture edits.

The two namespaces are not convenience aliases. An implementer imports traits from `node` and
supporting values from `node_support`. The API crate does not re-export either namespace at its
root.

### Node build context

`NodeBuildContext` is the narrow service contract passed to `RuntimeBuilder`. It replaces
`CompileCtx` in every plugin-visible signature. It exposes only operations required while a
concrete node is materialized, including derived-lane access, retention and persistent-cache
configuration, waveform/table presentation registration, and runtime sampling activity lookup.

The compiler owns the concrete context state and implements `NodeBuildContext`. Host-only result
operations, such as taking resolved sampling candidates or publishing the final presentation
registries, remain on `CompileCtx`, which is exposed through the compiler crate root.
A plugin cannot receive or import that concrete context through the graph-node API.

### Graph compiler facade

`logic_analyzer_graph_compiler` owns graph lowering, validation, discovery, execution, cache planning, and
saved-graph synchronization. Its crate-root facade is consumed by `logic_analyzer_ui`, native and
web composition, headless hosts, and integration tests. Graph-node contracts are imported directly
from `logic_analyzer_graph_api` rather than forwarded through the compiler crate.

`GraphCompiler` owns the inventory-derived builder registry and provides these application-facing
operations:

- lower and validate a graph;
- discover capture and trigger features;
- apply a feature edit through the owning node;
- resolve sampling overlays and cache plans;
- synchronize saved payload subscriptions;
- start or update an application run and live analysis.

The application UI constructs the editor's `NodeTypeRegistry` directly from the validated
`GraphNodeRegistration` inventory. The compiler consumes the same validated inventory only to
construct runtime builders and does not expose an editor-registry operation.

Viewer-output discovery, checkbox state, legacy `show_in_view` migration, and persistence of the
`logic_analyzer_graph.viewer_selections` extension are UI-owned. The compiler facade exposes no
viewer-selection operations. During the transition, lowering still reads the saved selection
manifest to synthesize its internal collection subscription; removing that read and the synthetic
Viewer node is tracked by the proposed-future migration below.

Compiler result types belong to the crate-root facade: `CompiledGraph`, `CompiledNode`,
`CompiledEdge`, `CompileError`, `ApplyError`, `LiveRun`, discovered feature wrappers,
compatibility warnings, and resolved sampling candidates.

Node-supplied descriptions and host-resolved results remain distinct:

| Node API description | Host result |
| --- | --- |
| `SamplingOverlayDescriptor` | `SamplingOverlayCandidate` |
| `TriggerConfigurationFeature` | `DiscoveredTriggerConfiguration` |
| `CapturePresentation` | `DiscoveredCapturePresentation` |
| `DecoderTableColumnPresentation` | `DecoderTableSource` |
| `RuntimeBuilder` | `CompiledNode` |
| `LiveCaptureFeature` | `DiscoveredLiveCaptureFeature` |

### Built-in graph nodes

`logic_analyzer_graph_nodes` contains only built-in graph-node features and their atomic payload
capabilities. Each node directory owns its definition, state, migration, builder, presentation
metadata, inventory submission, and isolated test. Concrete node symbols do not leave their
directory facade.

Built-in socket types and built-in collected-payload presentations live with the built-in node
bundle. The bundle submits registrations defined by `logic_analyzer_graph_api`; it does not call
compiler registration functions. The application references a small linker anchor from every
enabled built-in or plugin crate before inventory is read on native and wasm.

### Capture export

`logic_analyzer_capture_export` owns streaming export of finalized capture storage. It depends on
`signal_processing` capture contracts and format libraries, not on graph compilation or graph
nodes. Native UI services call its explicit exporter interface. Unsupported targets exclude the
complete exporter implementation at the crate boundary.

### Test support

`logic_analyzer_test_support` owns deterministic acquisition providers used across crate
boundaries. It depends only on generic `signal_processing` capture contracts. Processing
conformance tests, compiler tests, built-in test nodes, and UI integration tests consume this owner
directly. Node-isolation mocks remain private to the built-in graph-node crate unless another crate
has a documented need for them.

### Inventory and composition

The inventory collection types are declared by `logic_analyzer_graph_api`. Built-in nodes and
plugins submit registrations there. `logic_analyzer_graph_compiler::GraphCompiler` reads those
registrations without importing any submitter.

Every enabled submission crate exposes an idempotent `link()` anchor. Native and web application
composition references those anchors before constructing `GraphCompiler`. The anchor exists only
to make linker retention explicit; registration remains inventory-driven and contains no manual
per-node list.

### Compatibility and saved graphs

Moving Rust symbols does not change stable graph-node IDs, payload IDs, builder names, serialized
node definitions, or namespaced graph extensions. Compatibility remains owned by each concrete
node migration. Generic API/compiler crates never translate concrete node names or state.

### Application-supplied output subscriptions

`logic_analyzer_ui` owns viewer-output selection discovery, editing, migration, and persistence.
It translates the selected graph endpoints into
`logic_analyzer_graph_compiler::OutputSubscriptionPlan` and updates the compiler before discovery,
lowering, execution, cache planning, or live graph updates. The plan contains only node/output
identities; it carries no widget, lane, renderer, or panel state.

The compiler uses the same plan for source-channel visibility and retained derived-output
collection. Its production code does not read the
`logic_analyzer_graph.viewer_selections` extension. Saved payload-identity reconciliation also
receives the plan explicitly, so compatibility metadata cannot silently become an alternate
source of current UI selections.

### Enforcement

Architecture checks enforce these dependency rules:

- graph API does not depend on the compiler, built-in nodes, processing implementations, UI, or
  capture export;
- compiler does not depend on built-in nodes, concrete processing nodes, UI, or export formats;
- built-in graph nodes and plugins do not depend on the compiler crate;
- UI does not import built-in node implementation paths or `logic_analyzer_processing` concrete
  nodes;
- capture export does not depend on graph crates;
- concrete graph-node facades do not re-export implementation symbols.

Native and wasm builds exercise the same inventory and public API surfaces. Target selection stays
at whole implementation-module and linker-composition boundaries.

## Proposed future: UI-controlled compiler boundary

`logic_analyzer_graph_compiler` will own only the graph-to-processing lifecycle. It accepts a
graph document, lowers it to a processing graph, executes that graph, and exposes the data and
source-readiness results produced by the run. It does not construct a node-graph widget, a logic
analyzer widget, waveform groups, table panels, renderer objects, or UI-selection state.

The compiler continues to consume the `node_graph` document model because that model is its input
format. It does not consume `NodeGraphWidget`, `egui`, or `logic_analyzer_viewer`. Node catalog
construction belongs to built-in-node or application composition. The UI owns graph editing and
converts its editing state into the graph document passed to the compiler.

The UI controls subscriptions through the existing generic subscription plan. The compiler will
materialize those subscriptions without constructing a synthetic Viewer node and return a
run-data handle containing the retained derived lanes, collected table data, run diagnostics, and
source-readiness artifacts. The UI will bind those results to its waveform and table widgets after
the run starts; it may attach, detach, or rebind its own views without making the compiler invoke a
UI callback.

File and live sources report their viewer-usable data through the same application-neutral
source-readiness result. For a file source, preparation completes preload, cache lookup or
creation, and indexing before publishing the available capture data. For a live source, the run
publishes its cache and index as they become available. The compiler owns orchestration and its
explicit cache-directory configuration; source and storage implementations own their platform
details.

The resulting dependency direction is:

```text
node_graph document ──> logic_analyzer_graph_compiler ──> signal_processing
                              │
                              └──> run data and source readiness

logic_analyzer_graph_nodes ──> logic_analyzer_graph_api
logic_analyzer_ui ──> logic_analyzer_graph_compiler, logic_analyzer_graph_nodes,
                      logic_analyzer_viewer
```

No compiler-to-UI callback trait is required. A callback would reverse lifecycle ownership and
make subscription, teardown, and presentation state implicit. The UI creates subscription plans,
starts or updates runs, and consumes run data through explicit compiler operations.

Saved graphs remain compatible through explicit UI-owned migration. Existing Viewer-node state and
the `logic_analyzer_graph.viewer_selections` extension migrate to UI subscription state with a
user-visible warning; the compiler does not preserve this compatibility through viewer-specific
branches.
