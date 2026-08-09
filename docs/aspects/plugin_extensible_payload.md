# Plugin-Extensible Payload and Presentation Design

## Plug-in composition

Compile-time plug-in crates submit payload, graph-node, and UI-panel capabilities through
`inventory`. The host retains each enabled bundle through its idempotent `link()` anchor, then
applies payloads, graph nodes, and panels in deterministic stable-ID order. Native and web hosts
reference every enabled anchor; the web entry point invokes module constructors once before it
constructs the application so inventory submissions are available.

[example-plugin](../../plugins/example-plugin) demonstrates custom channel and payload types,
socket types, graph definitions and runtime builders, a waveform renderer, and an independently
openable panel.

## Architecture

`PortKind` is an open runtime payload identity. A compile-time plugin implements `PortValue` for a
Rust payload type it owns. When a plug-in uses a lower-level processing crate that owns the type,
its graph crate constructs the same identity with `PortKind::of_named`; this avoids making
processing depend on graph contracts merely to satisfy trait coherence. A graph-node inventory submission
carries any idempotent runtime channel setup needed for a non-collected custom payload; collected
payload capability registration performs the same typed channel setup as part of its atomic
submission.

`PayloadRegistry` records a durable, plugin-owned identity for each payload intended to
become collectable. `PayloadRegistration` inventory submissions atomically provide that
identity, typed channel setup, adapter factory, request configuration, persistence policy, and
default waveform presentation. `logic_analyzer_graph_registry::GraphRegistry` applies submissions in stable-ID
order and rejects identity/type collisions before graph-node payload requirements are validated.

`DerivedDataCollector` schedules adapter-created lane ingestors beside its
other lanes. An adapter publishes its retained, type-erased query object through `DerivedLanes`,
so a later subscriber can discover it by stable payload identity and downcast only to its own
registered query type. Built-in payloads are registered through this same path and retain their
digital, indexed-word, timestamp-event, numeric, text, and protocol-packet data behind their
adapters.

`GraphRegistry` owns one collection-subscription contract per subscribable payload. The contract
binds the open `PortKind` to its adapter descriptor, diagnostic name, default waveform
presentation, request configuration, and optional persistent-cache policy. Lowering obtains the
accepted kinds from these contracts. Materialization invokes the selected contract and adapter;
the generic data-collector builder contains no built-in payload list, type comparison, adapter
registry construction, or payload-specific request setup. Registering a payload identity or an
adapter alone therefore does not make it subscribable.

The built-in digital adapter publishes an opaque query with bounded exact-transition and dense
activity snapshots, cursor-boundary lookup, and timeline extent. Its viewer presentation consumes
those snapshots through the same renderer contract as a plugin payload.

The generic timestamp-event adapter provides the same query capabilities with exact event
timestamps or dense event-activity snapshots. The built-in trigger payload registration binds
that neutral adapter to trigger presentation metadata; neither the generic collector nor the
generic viewer interprets trigger identity to render the normal subscribed row.

The built-in numeric and text adapters retain their respective `i64` and `String` payload values in
their queries. Their graph-owned presentation adapters format bounded values only while rendering;
the generic collector does not convert a payload into display text.

The built-in protocol-packet payload feature dispatches each retained `ProtocolPacket` through a
compile-time formatter registration keyed by its exact `protocol_id`. Built-in protocol features
register only a bounded display projection; the graph-owned packet renderer retains ownership of
timeline geometry, clipping, and drawing. A missing or ambiguous registration uses a bounded
protocol-neutral value formatter, so mixed-protocol lanes remain readable without name-based
cases in the generic viewer or registry.

The built-in word adapter exposes `CollectedWordLaneQuery` as its concrete query contract. `Word`
represents both ordinary numeric decoder values and arbitrary-width byte values, with optional
text supplied by the decoder. Its indexed storage, waveform snapshots, and table rows preserve
that complete value. Generic subscribers use the bounded waveform and table capabilities through
`CollectedLaneQuery`; concrete word diagnostics may downcast the query and inspect its indexed
store without accessing generic collector storage.

`CollectedLaneQuery` supplies an immutable snapshot only when its payload has waveform
semantics. The request is bounded by a visible time window and item limit. The viewer passes the
returned `OpaqueCollectedLaneSnapshot` to the payload's renderer only after it has released
retained-data locks; the renderer downcasts the snapshot only to its own registered type. An
opaque lane activates an explicit viewer group or a singleton default presentation registered for
its stable payload identity. An optional snapshot generation lets the viewer reuse immutable
results until adapter-visible data changes; adapters without this contract remain uncached.
Renderers explicitly advertise and project bounded snapshots into payload-neutral level and event
transitions for hover measurement and event-row interaction. Cursor boundaries,
timeline extent, and live status are query capabilities. The generic viewer neither reads a
parallel built-in lane representation nor matches a payload type.

## Payload adapters

A payload is *collectable* only when its owner registers explicit timeline and storage semantics.
This is intentionally narrower than being a `PortValue`: a payload such as a configuration command
may flow through a graph but has no useful retained timeline representation.

```text
plugin payload T
      │
      ├── PortValue + runtime channel registration
      │
      └── payload adapter registration
                   │
                   ▼
           PayloadRegistry
                   │
                   ▼
           DerivedDataCollector
                   │
                   ▼
           retained query handle
             ├── waveform presentation
             ├── table presentation
             └── plugin panel
```

The collector owns no protocol, payload, viewer, table, or panel knowledge. It schedules a set of
object-safe lane ingestors. Each ingestor is constructed by the adapter registered for one payload
type.

The `signal_derived::derived_data_collector` private facade reflects those ownership boundaries:

```text
derived_data_collector/
  mod.rs       supported crate facade
  catalog.rs   payload-neutral published-lane discovery
  collector.rs payload-neutral scheduling, retention, and metrics
  storage.rs   generic in-memory storage accounting
  digital.rs   digital snapshot, fold, store, query, ingestor, and adapter
  number.rs    numeric snapshot, fold, store, query, ingestor, and adapter
  text.rs      text snapshot, fold, store, query, ingestor, and adapter
  trigger.rs   marker snapshot, fold, store, query, ingestor, and adapter
  word.rs      word snapshot, fold, memory/indexed stores, query, ingestor, and adapter
```

Payload modules depend on the shared policy and catalog contracts. The shared modules do not
import built-in payload types. Tests construct built-in ingestors through `PayloadAdapter` and
`CollectedLaneRequest`, matching production construction; focused storage and query tests may use
crate-private payload-module fixtures without widening the crate facade.

The runtime channel registry creates both the typed receiver and a type-erased readiness handle.
The cooperative runner schedules that handle without comparing payload `TypeId`s, so any
registered plugin payload has the same native and single-threaded execution path.

### Stable identities

An adapter combines the registered process-local Rust `TypeId` and stable payload identifier such
as `"org.logicconduit.camera-frame/v1"` with its typed ingest and presentation contracts.

`TypeId` selects the typed channel and adapter while an application runs. Saved graphs, saved
panel state, persistent caches, and missing-plugin diagnostics use the stable identifier; they
never serialize `TypeId`.

Graph documents store viewer choices in the versioned
`logic_analyzer_graph.viewer_selections` extension and store a
`logic_analyzer_graph.payload_subscriptions` entry for every explicit Viewer input and selected
output. Each payload entry identifies its endpoint and the payload owner's stable identifier.
The generic graph model preserves both namespaced extensions without interpreting them. On
load, persisted built-in lanes without a stable payload identity receive their registered
identity without changing
their connections, selection state, ordering, grouping, badge, or renderer. The application shows
a persistent compatibility warning when a saved payload, ingestion subscription, or presentation
registration is unavailable and retains the unresolved identity on subsequent saves. The owner
leaves an invalid or unsupported extension schema unchanged; it does not partially migrate or
rewrite a version it cannot understand.

The identity registry accepts an identical repeat registration, but rejects a Rust type assigned a
different identifier or an identifier assigned to a different Rust type. Adapter registration
uses the same rule for its storage and presentation definition. This prevents two
plugins from silently assigning incompatible semantics to one payload.

### Runtime adapter contract

`PayloadAdapter` registration for `T` creates a typed lane ingestor while exposing only
an erased, object-safe interface to the collector.

Adapter construction returns `PayloadIngestorConstructionError`. Plugin adapters retain typed
construction causes through its source-bearing variant; the explicit diagnostic adapter marks the
point where an external implementation can supply text only. The generated collector adds payload
and lane context without formatting the adapter error.

```rust
trait ErasedLaneIngestor: Send {
    fn input_schema(&self, index: usize) -> PortSchema;
    fn ingest(&mut self, input: &InputPort) -> WorkResult<usize>;
    fn is_finished(&self) -> bool;
}
```

The typed factory captures `T` during plugin registration. It creates the correct `PortSchema`,
downcasts the `InputPort` only to `Receiver<T>`, and owns all append state. The generic collector
only schedules ingestors and applies backpressure and retention policy. The ingestor publishes its
query handle while it is created, allowing subscribers to appear after data has been collected.

`CollectedLaneQuery` exposes bounded visible-window snapshots. Snapshots are immutable and
type-erased at the generic boundary. Plugin-owned presentation adapters receive only the snapshot
type registered with their payload; the generic viewer never gives them access to mutable
collector state. Timeline extent, activity summaries, and boundary snapping are added as explicit
capabilities rather than inferred from a payload type.

### Presentation and panels

The waveform viewer owns a separate presentation registry keyed by the payload identity.
Its adapter supplies default group/badge metadata and a renderer for bounded snapshots. Rendering
occurs after the retained-lane lock is released. `ViewerLaneTheme` supplies current background,
foreground, muted, accent, and error roles. `ViewerLaneInteractionContext` supplies the bounded
visible range, item budget, hover state, and optional pointer time. Renderers receive these value
objects instead of viewer internals and can therefore respond to host theme and interaction state
without reaching into `LogicAnalyzerViewer`.

Table projection is optional adapter metadata. `CollectedLaneQuery::table_metadata` supplies a
revision and row count for cache invalidation, while `table_snapshot(max_rows)` supplies bounded
scalar rows with a format hint and a completeness flag. The decoder-table panel consumes this
contract through opaque lane handles and never reads a concrete payload's retained storage. An
adapter may expose rows and columns when that is meaningful; no table-specific behavior is
required for arbitrary payloads.

Extra panels are UI-owned plugin registrations, not graph or processing registrations. A panel
submits a stable identity, title, icon, minimum size, factory, and optional singleton constraint
through `UiPanelRegistration`. The application discovers those descriptors when it
builds the View menu and panel-layout catalog, so adding a panel does not add an application dispatch
branch. Each panel instance receives a restricted read-only context containing collected-lane
descriptors and query handles. It keeps versioned serializable state under its stable panel identity
and the layout instance identity. A panel can be opened after a capture finishes because it queries
retained data rather than receiving the live stream. The current contract intentionally exposes no
application mutation commands. Registration validation returns the UI-owned
`PluginPanelRegistrationError`, while state restoration returns the source-bearing
`PluginPanelStateError`. The application retains restoration failures through panel lookup and adds
the panel title only at toast presentation.

The out-of-tree example plugin proves the complete route with `CameraFrame`: a custom socket and
finite source produce timestamped RGB images, a custom adapter retains bounded frames, a viewer
renderer draws bounded thumbnail snapshots, and a Camera Frames panel queries the same retained
lane. The source reaches collection through an explicit Viewer connection; neither the collector,
viewer, application panel catalog, nor panel layout contains a CameraFrame-specific branch.

Contract tests cover identity and adapter collisions, missing registrations, typed channel
construction and negotiation, retention limits, bounded dense snapshots, timeline extent, renderer
lock release, and saved-panel-state diagnostics. Architecture tests reject built-in payload and
protocol checks in generic collection, compiler, and viewer paths. CI compiles the example plugin
on native targets as part of the workspace and explicitly on `wasm32-unknown-unknown`.

### View-lane ownership

Payload owners declare presentation through stable, protocol-neutral metadata. A presentation
selects either a default singleton row or an explicit compound group containing ordered tracks.
Group and track identities are opaque keys, not display strings; graph lowering namespaces local
keys with the producer and output identity so independently configured instances cannot collide.

The compiler preserves this metadata without inspecting its values. The UI binds it to the viewer
presentation registry, which owns row geometry, ordering, renaming, clipping, time transforms,
and bounded interaction. A renderer receives only a bounded immutable snapshot, a theme, and a
value interaction context after retained-lane locks have been released. It may format concrete
values, select eligible snap boundaries, and choose track geometry, but it performs no storage I/O
and cannot access mutable viewer or collector state.

The standard payload registrations provide digital, word, marker, numeric, and text singleton
presentations. A concrete decoder can instead register a compound presentation: UART contributes
bit-detail and frame tracks; SPI contributes independent MOSI and optional MISO groups with bit
and data tracks. These are concrete adapter decisions, so generic graph, compiler, and viewer
code never identifies decoder names, socket labels, protocol sentinel values, or display text.

Presentation metadata is derived from current node definitions and renderer registrations, not
serialized renderer objects. Saved documents persist stable payload identities and selected output
subscriptions. If a feature changes its state or socket schema, that concrete feature migrates the
document at its load boundary and emits a user-visible warning; generic viewer and compiler code
does not repair protocol wiring.

### Crate ownership

- `signal_derived` owns type-erased ingestion, retained query, snapshot, and storage contracts.
- `logic_analyzer_viewer` owns presentation adapters and drawing contracts for those snapshots.
- `logic_analyzer_graph_compiler` owns compiler negotiation: it accepts only registered collectable payloads
  for a data subscription and reports a targeted error for an unavailable subscription contract
  or adapter.
- `logic_analyzer_ui` owns panel factories, panel state, and the read-only panel data context.
- Application composition iterates the independent graph, payload, and panel inventories without
  making `signal_derived` depend on graph or UI crates. Enabled plugin crates are retained by a
  host symbol anchor; the web platform entry point invokes module constructors once before the
  first inventory iteration.

Plug-ins are linked Rust crates retained by an application-owned symbol anchor. Inventory is read
only after every enabled plug-in anchor has been invoked.
