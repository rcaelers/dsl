# Application Composition Design

Design of application-facing orchestration in `logic-analyzer-ui`
([crates/logic_analyzer_ui](../../crates/logic_analyzer_ui)). It composes the editor, viewer,
graph-service port, and host services into one portable application. The native and web binaries
are documented separately in [Application Shells Design](../crates/application_shells.md).
Companion designs cover [graph composition](graph_composition.md),
[processing graph workflows](processing_workflows.md),
[the node-graph widget](../crates/node_graph.md),
the `signal_runtime` Rustdoc, and
[the logic-analyzer viewer](../crates/logic_analyzer_viewer.md).

Layering rule: `node_graph` stays UI-generic, the `platform_*` foundations and `signal_*` contract
owners stay generic and UI-free, and the capture-format, device, protocol-decoder, transform, sink,
and generator crates own UI-independent runtime behavior. The UI consumes graph-service and
host-service contracts; it does not define concrete nodes or compiler behavior.

## Application services

The application window uses a persistent panel layout. The default places the Logic Analyzer above
the Node Graph; auxiliary panels occupy a right column when opened. Users can close any panel and
restore it through the View menu without changing the saved layout of the other panels.

The View menu restores the primary Logic Analyzer and Node Graph panels, and opens auxiliary Log,
Memory, Watches, Triggers, Decoder, and registered plug-in panels. Reset Layout restores the
default primary-panel arrangement. The macOS menu bar exposes the same panel commands.

The Memory panel is the application-owned resource inventory. The application retains the same
`DerivedLanes` query handle that it supplies to presentation consumers and records generic capture
storage metadata when it attaches a prepared raw capture. It combines those facts with
platform-neutral storage snapshots published by payload adapters, statistics from the
application-owned decoded-block cache handle, and persistent-cache inspection through the graph
service. It identifies active services, raw-capture backing, every collected signal lane,
decoded-block LRU usage, and the current graph's persistent derived-cache entries.
Presentation widgets do not define or aggregate memory diagnostics. Generic UI code displays
payload IDs and storage contracts; it does not infer protocols or concrete nodes from their names.

- `App::build` creates the editor node-type registry from the editor inventory after matching it
  to headless graph features by stable ID, composes the
  UI-owned graph and host service ports, and installs host-supplied symbol fonts used by menu
  glyphs. The graph service's lowerer owns a validated
  `logic_analyzer_graph_registry::GraphRegistry` snapshot; its separate runtime has no registry or
  compiler dependency.
- Graphs are saved/loaded as the editor's JSON document (`⌘O` / `⌘S` / `⇧⌘S`).
- Every frame, the app asks the graph integration for an opaque pre-run capture presentation.
  Concrete source builders supply an indexed-capture factory, an in-memory preview, or a channel
  layout. The app and viewer do not identify node types or know what DSL and Sigrok paths mean.
  Runtime processing nodes remain graph-runtime-owned and are created only when a run starts.
- File commands include New, Open, Open Recent, Save, and Save As. Destructive actions over
  an unsaved graph share one save/discard/cancel guard; recent paths are deduplicated and
  persisted through injected host capabilities.
- Transient file/edit/live-run results use dismissible, self-expiring toasts; persistent run state
  remains in the toolbar. Every toast is also retained for the current session in the Log panel
  with its UTC time, severity, and user-facing source (`Global`, a panel, a node, or a node
  socket). The toolbar also shows the active pane's contextual hint.
- Run and Stop are shared guarded commands used by toolbar and menus. Their shortcuts are
  `Cmd/Ctrl+R` and `Cmd/Ctrl+.` respectively.

## Run lifecycle & live editing

Loading a graph first opens every valid persistent derived-data cache selected by that document
and binds those lanes to the viewer without materializing producers or sinks. This cached preview
is passive application state: it does not create an active run or produce side effects. Pressing
Run releases the preview handles, clears the graph's selected derived caches, and starts a fresh
execution which rebuilds them. Run therefore always means execute, while reopening a document is
enough to inspect previously completed derived data.

The graph service lowers the current document and passes the resulting `ProcessingGraph` to
`GraphRuntime::start`; document semantics are complete before materialization begins. The app
always runs through `LiveRun` machinery (a file replay is a run whose source finishes).

Per frame, the app calls `run.pump(budget)` (no-op on the native threaded manager; on wasm
this is what executes node `work()`s) and requests ~16 ms repaints while running so derived
lanes fill visibly. Every 500 ms it publishes per-node progress counters into node headers. The
app also compares a stable semantic graph snapshot with the snapshot last applied to the run.
This snapshot excludes editor-only position, selection, collapse, title, header-color, frame, and
allocation state. Only when the semantic snapshot changes does the app:

1. Re-lower the edited graph.
2. Call the graph service's `apply_run`, which diffs desired vs. running by `NodeId` and applies the
   cheapest edit class per difference:

| Edit | Action | Node threads touched |
|---|---|---|
| Add a tap (new branch on existing outputs) | materialize just the new nodes; subscribe; sticky levels prime | none |
| Remove a branch | unsubscribe + close its lists → branch-local shutdown cascade | removed ones |
| Hot prop change (matcher pattern, template, …) | `hot_config` → control-channel `Configure` | none |
| Prop change that can't hot-apply / rewire | restart-in-place | that node |
| Source changed / replaced | `NeedsFullRestart` — reported, run left untouched | — |

Mid-edit graphs are often momentarily invalid; compile errors during live sync are silently
ignored (the running pipeline continues; the diff retries once the graph is valid again).
`Disconnect` overflow events surface as warning badges ("can't keep up"). Stop = wind-down
via the manager; final progress counts stick. Each run swaps a fresh `DerivedLanes` store
into the viewer so stale lanes vanish atomically.

Node-contributed View panels also report state changes directly to the host. The app applies those
changes and rebuilds waveform/table presentation registries in the same frame rather than waiting
for the periodic live-edit pass. A builder's `execution_state` projection excludes
presentation-only fields such as `display_format`, so changing a format refreshes already-collected
lanes without restarting the decoder. Retention and presentation are separate contracts: every
collectable connected output, plus every view-capable output on a participating node, is retained
independently of its current visibility. Viewer lane selection only rebinds presentation metadata
to those cached lanes. The same metadata-only path applies while processing is active and after it
has completed; a View panel change never restarts or reruns the processing graph.

The lowerer/runtime handoff, worker path, cache behavior, and live-apply classifications are tested
at their owning crate boundaries and by the top-level integration package. UI tests replace the
private graph-service port with deterministic implementations.
