# Graph Composition Design

## Responsibilities

`logic_analyzer_graph_api` owns the compile-time plugin contract through its `node` and
`node_support` namespaces. Graph-node and payload inventory types are defined there;
inventory assembly is compiler-owned and consumes submissions without importing a node bundle.

`logic_analyzer_graph_nodes` owns the built-in node definitions, builders, migrations, socket
types, payload presentations, and inventory submissions. `logic_analyzer_graph_compiler` owns
compiler and application-facing services. Compiler tests inject local runtime builders directly;
full built-in-node and compiler composition belongs to the top-level integration-test package.
`logic_analyzer_capture_export` owns native streaming capture export without depending on a graph
crate. `logic_analyzer_test_support` owns deterministic capture providers shared by cross-crate
tests. Native and web application composition link the built-in bundle before constructing the host
compiler.

## Architecture

The graph domain is divided into crates whose dependency edges follow the direction of their
contracts:

```text
node_graph                 signal_processing       logic_analyzer_viewer
    ^                            ^                      ^          ^
    |                            |                      |          |
    +---- logic_analyzer_graph_api                     |          |
                    ^                                  |          |
          +---------+-------------------+--------------+          |
          |                             |                         |
logic_analyzer_graph_compiler   logic_analyzer_graph_nodes        |
          ^                             ^                         |
          |                             |                         |
          +---------- logic_analyzer_ui +-------------------------+

logic_analyzer_capture_export ---> signal_processing
logic_analyzer_test_support  ---> signal_processing
```

`logic_analyzer_graph_compiler` and `logic_analyzer_graph_nodes` both depend on
`logic_analyzer_graph_api`. The compiler crate never depends on the built-in-node crate. A plugin
depends on the API crate and the lower-level domains required by its own implementation; it does
not depend on the compiler or application UI.

### Graph API contract

`logic_analyzer_graph_api` is the supported compile-time extension contract. It supplies
directory-backed `node` and `node_support` namespaces; compiler, built-in-node, plug-in, and UI
composition depend on that contract without making the API depend back on any of them. Its detailed
ownership and supported surface are defined by the
[`logic_analyzer_graph_api` crate design](../crates/logic_analyzer_graph_api.md).

### Graph compiler facade

`logic_analyzer_graph_compiler` owns graph lowering, validation, discovery, execution, and cache
planning. Its crate-root facade is consumed by the UI's production graph-service adapter, native
and web composition, headless hosts, and integration tests. Graph-node contracts are imported
directly from `logic_analyzer_graph_api` rather than forwarded through the compiler crate.

`GraphCompiler` owns the inventory-derived builder registry and provides these application-facing
operations:

- lower and validate a graph;
- discover capture and trigger features;
- apply a feature edit through the owning node;
- resolve sampling overlays and cache plans;
- start or update an application run and live analysis.

The application UI depends on its private `GraphService` port for discovery, source preparation,
run control, and live edits, and on `GraphRun` for lifecycle, progress, readiness, and collected
results. The production adapter implements those ports for `GraphCompiler` and `LiveRun`; UI tests
substitute local deterministic implementations. The UI constructs the editor's `NodeTypeRegistry`
directly from the validated `GraphNodeRegistration` inventory. The compiler consumes the same
validated inventory only to construct runtime builders and does not expose an editor-registry
operation.

Resolved sampling overlays contain only raw-row identities and a shared sampling-point store
configured by the concrete runtime node. The UI adapts that metadata to the generic viewer, which
queries the visible time range and renders the records without reading raw channels, selecting
edges, evaluating qualifiers, or looking up sampled values. Every lowered overlay receives a store
and its runtime node records accepted points independently of current UI visibility; a node may
instead satisfy a transient store through its lazy indexed provider. For a stable capture identity,
the compiler assigns a protocol-neutral persistent cache key to every sampling overlay. The store
records accepted points in the same indexed artifact infrastructure as other derived data and
reopens them for cached preview without materializing or executing the decoder. Processing nodes
submit allocation-free packed records to a bounded writer; artifact encoding and repository I/O
run independently of the decoder's ordered merge. Cooperative hosts use the same format through a
direct writer. Dynamic live sources retain run-owned storage because they have no reusable capture
identity. Hiding an overlay does not stop collection or delete decisions, so transient panel
refreshes, hide/show operations, and application restarts are non-destructive.
The UI persists an ordered set of independently selected overlay node identities. Loading the
legacy single-node value migrates it to that set at the UI-owned document boundary and reports the
migration to the user.

Viewer-output discovery, checkbox state, legacy `show_in_view` migration, and persistence of the
`logic_analyzer_graph.viewer_selections` extension are UI-owned. The compiler facade exposes no
viewer-selection operations and receives current selection only through `OutputSubscriptionPlan`.

Compiler result types belong to the crate-root facade: `CompiledGraph`, `CompiledNode`,
`CompiledEdge`, `CompileError`, `ApplyError`, `LiveRun`, discovered feature wrappers,
and resolved sampling candidates.

Node-supplied descriptions and host-resolved results remain distinct:

| Node API description | Host result |
| --- | --- |
| `SamplingOverlayDescriptor` | `SamplingOverlayCandidate` |
| `TriggerConfigurationFeature` | `DiscoveredTriggerConfiguration` |
| `CapturePresentation` | `DiscoveredCapturePresentation` |
| `DecoderTableColumnDescriptor` | UI-owned decoder-table source |
| `LanePresentationDescriptor` | UI-owned viewer lane group |
| `RuntimeBuilder` | `CompiledNode` |
| `LiveCaptureFeature` | `DiscoveredLiveCaptureFeature` |

### Built-in graph nodes

`logic_analyzer_graph_nodes` contains only built-in graph-node features and their atomic payload
capabilities. Each node directory owns its definition, state, migration, builder, presentation
metadata, inventory submission, and isolated test. Concrete node symbols do not leave their
directory facade.

Built-in socket types, payload presentation descriptors, and concrete renderer
registrations live with the built-in node bundle. Neutral descriptors are submitted through
`logic_analyzer_graph_api`; renderer factories are submitted through the generic viewer registry
under the stable keys carried by those descriptors. The bundle does not call compiler or UI
registration functions. The application references a small linker anchor from every enabled
built-in or plugin crate before inventory is read on native and wasm.

### Capture export

`logic_analyzer_capture_export` owns streaming export of finalized capture storage. It depends on
`signal_processing` capture contracts and format libraries, not on graph compilation or graph
nodes. Native UI services call its explicit exporter interface. Unsupported targets exclude the
complete exporter implementation at the crate boundary.

### Test support

`logic_analyzer_test_support` owns deterministic acquisition providers used across crate
boundaries. It depends only on generic `signal_processing` capture contracts. Processing
conformance tests and built-in test nodes consume this owner directly. Compiler and UI tests use
locally owned fakes at their respective service boundaries. Node-isolation mocks remain private to
the built-in graph-node crate unless another crate has a documented need for them.

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
node definitions, or namespaced graph extensions. Concrete node-state compatibility remains owned
by each node migration. The UI explicitly migrates legacy Viewer nodes and `show_in_view` socket
state into `logic_analyzer_graph.viewer_selections`, rewrites Viewer-input payload identities to
their source-output identities, removes the obsolete nodes, and emits user-visible warnings. It
preserves unavailable plugin payload identities in
`logic_analyzer_graph.payload_subscriptions`. The compiler has no saved-Viewer module or facade
operation.

### Application-supplied output subscriptions

`logic_analyzer_ui` owns viewer-output selection discovery, editing, migration, and persistence.
It translates the selected graph endpoints into
`logic_analyzer_graph_compiler::OutputSubscriptionPlan` and updates the compiler before discovery,
lowering, execution, cache planning, or live graph updates. The plan contains only node/output
identities; it carries no widget, lane, renderer, or panel state.

The compiler uses the same plan for source-channel visibility and retained derived-output
collection. Selected derived outputs lower to a compiler-owned `Output Subscription Collector`;
the compiler never synthesizes or identifies a concrete Viewer node. Its production code does not read the
`logic_analyzer_graph.viewer_selections` extension. Saved payload-identity reconciliation is also
UI-owned, so compatibility metadata cannot silently become an alternate source of compiler
subscriptions.

Each run exposes its collected output subscriptions with stable runtime lane names and resolved
producer metadata. The metadata contains generic grouping, ordering, track, badge, and stable
renderer-key descriptions; it contains no viewer objects or renderer trait objects.
`logic_analyzer_ui` resolves the keys through the viewer's plugin registry and translates the
descriptions into waveform groups and tracks. Missing renderer registrations are explicit UI
binding errors. Live graph updates publish replacement subscription metadata before the UI
rebinds presentations. The compiler only transports the descriptors.

The UI retains a presentation catalog separately from the run-owned lane data and projects that
catalog through the current graph's stable node and output identities. Deleting a producer node
therefore removes its waveform and table presentations immediately without deleting cached data.
Undoing or redoing the graph edit changes the projection again, so a restored node can reuse the
same retained lane without rerunning the processing graph. New or reconfigured live outputs merge
their replacement metadata into the catalog by stable source endpoint.

`RunData` is the compiler-owned application-facing snapshot for both initial materialization and
live runs. It consolidates retained `DerivedLanes`, output and table subscriptions, sampling plans,
shared run diagnostics, and shared source readiness. Source readiness identifies file or live data
and independently reports preload, cache, index, and consumable-data artifacts as pending,
available, unsupported, or failed. The shared registries provide the explicit publication
boundary; consumers read snapshots and remain free to
attach or replace views after a run has started.

Source builders declare `SourceDataLifecycle` rather than relying on node names. Native file
sources declare preload, cache, and index capabilities; their wasm implementations declare the
same file lifecycle with filesystem-backed cache and index capabilities unsupported. Native live
sources declare cache and growing-index capabilities, while unavailable wasm hardware
implementations declare those artifacts unsupported. Source materialization publishes supported
file artifacts as pending. Compiler-owned preparation opens the source factory off the UI thread
and produces a completed format-neutral capture index; the UI attaches that index and marks the
supported file artifacts available in the run registry. The native capture coordinator publishes
live cache and growing-index availability as each artifact becomes attachable.

Sampling overlays follow the same boundary. `signal_processing::SamplingPointStore` is the
neutral run-owned cache; the compiler resolves only the clock and sampled inputs to capture rows
and gives the shared store to the concrete runtime node. Every run retains the sampling decisions
independently of whether the corresponding overlay is visible, so attaching an overlay after the
run uses existing data and never requires re-execution. The node either records decisions while
processing or supplies a lazy `SamplingPointProvider` backed by input indexes and sparse derived
control queries. Dense captures therefore do not require a second capture-sized vector merely to
retain sampling markers. Persistent producers fill an opaque storage-ready batch owned by the
neutral sampling contract, allowing queued publication without a producer-owned intermediate
point vector or a second conversion pass. The UI controls only which shared stores are presented,
converts their row identities into the logic-analyzer widget's passive overlay type, and never
changes collection policy. The compiler does not construct a widget overlay, and the widget does
not reinterpret raw capture channels.

Decoder-table subscriptions are published as collected table lanes with their resolved producer
metadata. The UI owns the resolved decoder-table source, column, and registry models, resolves
renderer keys, and groups and orders the lanes. Consequently neither the graph API nor the
compiler owns panel-facing lane IDs, track IDs, renderer objects, or registry mutation.

`logic_analyzer_graph_api` has no dependency on `egui` or `logic_analyzer_viewer`. Concrete node
features may implement viewer renderers because protocol-specific rendering belongs with those
features; the stable-key registry keeps that implementation out of generic graph contracts.

`logic_analyzer_graph_compiler` has no production dependency on `egui` or
`logic_analyzer_viewer`. It does not own a waveform presentation registry or invoke presentation
callbacks while materializing processing nodes. The UI removes legacy Viewer nodes during document
migration and constructs a fresh presentation registry from the resulting subscription set.
Its dependency on `node_graph` is restricted to the supported `node_graph::api`
document, identifier, connection, and socket contracts; it does not import the widget or editor
runtime surface.

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

## Lowering and materialization

Every executable node feature submits its `GraphNodeRegistration` and feature-local
`RuntimeBuilder` through `inventory`, using its stable graph-node ID as the host-composition
identity. `logic_analyzer_graph_compiler` validates those submissions and constructs its private
builder registry without importing a built-in node bundle or maintaining a hand-written catalog.

`PortKind` is an open, `TypeId`-backed payload identity (`PortKind::of::<T: PortValue>()`,
[port.rs](../../crates/logic_analyzer_graph_api/src/node_support/port.rs)) — the compiler-layer
analogue of `node_graph::SocketDef` and `signal_processing::register_type`, so plugin crates add
payload types without editing compiler code.

Kind negotiation is per edge: `offered(producer) ∩ accepted(consumer)`; empty produces a compile
error and the producer's preference order resolves multiple matches. One UI socket can therefore
lower to different concrete runtime ports for downstream consumers that accept different payload
kinds. UI-only frames and reroutes have no builder; lowering follows their graph connections.

Lowering produces a `CompiledGraph` containing canonical node state, resolved input kinds, stable
runtime names, and edges with concrete port names and buffer sizes. It prunes to sink-reachable
nodes, validates required inputs and state, and requires exactly one time-domain source. Errors
carry `NodeId` so the editor can badge their owner.

Materialization builds each runtime node, configures generated collectors and cache-backed stores,
and feeds `NodeSpec`s to the host-selected `AppManager`. Native composition supplies its threaded
backend and web composition supplies its cooperative backend through the same factory contract.
Direct use of `signal_processing::Pipeline` remains a separate lower-level API for headless and
runtime-specific consumers.

Buffer size comes from the consumer edge's `PortKind` (`PortValue::buffer_size`):

| Kind | Buffer | Rationale |
|---|---|---|
| `Block` | 4 | each block is approximately 2 MB |
| `SampleEdge`, producer is a source | 10,000,000 | RLE edge bursts of fast raw channels |
| `SampleEdge`, control path | 1,000 | low rate |
| everything else (`Word`, `Trigger`, `Number`, `Text`, …) | 100 | sparse events |

Sizes reflect item characteristics rather than inter-branch skew. The explicit `Buffer` node owns
intentional decoupling and supplies its own input-capacity override. The runtime flow-control rule
is defined by the `signal_processing` runtime Rustdoc.

## Source-readiness orchestration

File and live sources report their viewer-usable data through the same application-neutral
source-readiness result. For a file source, preparation completes preload, cache lookup or
creation, and indexing before publishing the available capture data. For a live source, the run
publishes its cache and index as they become available. The compiler owns orchestration while
source and storage implementations own their platform details. The UI remains in control of when
it polls preparation, attaches completed data, and reflects completion in the active run's
readiness registry.

Indexed preparation is submitted through a compiler-owned task-executor contract. The native
adapter runs the source-provided `CaptureIndexFactory` on a named worker, while deterministic test
executors can retain, complete, fail, or disconnect the same task without threads, clocks, or
production capture files. Dropping a pending task detaches its eventual result when the source is
reset or replaced.

Derived-cache discovery and cleanup use a separate compiler-owned backend contract. The native
adapter asks `signal_processing` to validate persistent annotation stores and enforce the cache
budget; the compiler sees only hit, miss, or unreadable availability. Only a confirmed hit removes
the corresponding producer path. Missing or corrupt data remains connected and is regenerated,
and cleanup failure does not change graph correctness. Persistent-store publication, cancellation,
corruption recovery, and filesystem cleanup remain owned and tested by `signal_processing`.

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
