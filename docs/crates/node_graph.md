# `node_graph` Design

Design of the `node-graph` crate ([crates/widgets/node_graph](../../crates/widgets/node_graph)): a reusable,
Blender-style node editor widget for egui. The crate is UI-generic — it knows nothing about
logic analyzers or pipelines; the application (see [Application Composition Design](../architecture/application_composition.md)) defines the
node types and compiles the drawn graph into something executable.

The widget's public API and node-definition contracts are documented at the `node_graph` crate
root and its `node_graph::api` facade. Portable document records are owned by
[`node_graph_document`](node_graph_document.md) and re-exported here for widget consumers.

---

## Layering

```text
crates/node_graph_document/src
└──            Serializable document: GraphState, Node, Socket, Connection, Frame

crates/widgets/node_graph/src
├── model/     Compatibility facade over node_graph_document
├── api/       Node-type definition API: NodeDef, InputDef/OutputDef, PropDef,
│              SocketDef, InlineControl, builtin socket/value types
├── runtime/   NodeTypeRegistry + type-erased per-node instances (TypedNode)
├── widget/    NodeGraphWidget: rendering, interaction, menus, panel, minimap
└── support/   View transform (pan/zoom), paint helpers
```

The dependency direction is strict: the document crate depends on no workspace crate, `api`
materializes document sockets, `runtime` erases `api` definitions into instances, and `widget`
orchestrates the editor layers while mapping neutral colors and positions to egui.

### Document model and editor instances

`GraphState` is the *document*: plain serde-serializable data (nodes, sockets, connections,
frames) with no trait objects or egui values. The persisted generic presentation values needed by
the editor live in the document crate, so a saved file round-trips without consulting any registry.
The widget converts `GraphColor` and `GraphPosition` at its egui boundary.

Each node additionally has a **runtime instance** (`Box<dyn NodeInstance>`, kept in a side
map on the widget): the typed node state plus its def's behavior (`on_update`, controls,
badge). Instances are rebuilt from the registry whenever a graph is loaded, undone, or
pasted (`restore_node`); the model's `node.state: serde_json::Value` is the single durable
representation of node state.

## The definition API

A node type is a `NodeDef` implementation: an associated serde `State` type plus static
descriptions of sockets, inline props, built-in Node-panel sections, node-contributed panel
presentations, and two hooks:

- `on_update(state, inputs, outputs)` — runs after any state edit or connect/disconnect;
  mutates socket visibility/styling (dynamic sockets).
- `badge(state)` — recomputed after every update; a validation/status message drawn under
  the node (`NodeBadge` with info/warning/error severity).

Socket types are `SocketDef` implementations (name, color, shape); `SocketWithControlDef`
additionally binds a value type that renders as an inline control while the socket is
unconnected. Builtins: `Bool`, `Int`, `Float`, `Str`, `File`, `Any`, with value types
(`IntValue`, `FloatValue`, `BoolValue`, `StringValue`, `FileValue`, `EnumValue`).
`EnumValue` persists by variant *name*, not index, so save files survive variant reorders.
`FileValue` uses the injected `FileDialogService` for immediate native dialogs or asynchronously
completed browser selection. A dropped file is offered to the same service when it lands on the
control. Save availability is independent from open/import availability, so a web host can import
capture bytes without advertising an output destination.

`NodeTypeRegistry::register::<T>()` records the def and auto-collects every socket type it
mentions (inputs, outputs, `.accepts::<T>()`) into a **type identity table**
(`socket_types: name → (color, shape)`; first registration wins). That table is what
re-skins resolved sockets and wires; compatibility checking never consults it.

`NodeTypeRegistry::register_with_state_update::<T, _>()` additionally binds one instance-owned,
type-safe state-update closure to that registration. The hook receives only the concrete node
state and generic sockets; the widget does not know why the state changes. Application composition
uses this seam for host-discovered metadata while leaving `NodeDef::on_update` deterministic and
host-independent. Each registry owns its closures, so two widgets can bind different services to
the same node definition without shared configuration.

## Socket type system

Connection validity is decided **per node, not per type** — there is no global cast table.
Each input declares which types it accepts beyond its native one
(`InputDef::new::<Signal>("Threshold").accepts::<Float>()`); the node's own processing is
responsible for handling any accepted type.

All data lives in the serialized `Socket`:

```rust
pub struct Socket {
    pub name: String,
    pub type_name: String,             // native type
    pub color: GraphColor,             // neutral idle look (def-controlled)
    pub shape: SocketShape,            // idle look
    pub allowed: Vec<String>,          // extra accepted type names
    pub resolved_type: Option<String>, // set while connected to a non-native type
    pub def_index: usize,              // which Input/OutputDef this socket came from
    pub variadic: Option<VariadicInfo>,
    pub visible: bool, pub hidden: bool, pub has_control: bool,
}
```

Key rules:

- **Compatibility**: `compatible(out_type, input) = out_type == "Any" || input.type_name ==
  "Any" || out_type == input.type_name || input.allowed.contains(out_type)` — see
  `Socket::accepts`. Checked at wire completion and drag snapping.
- **Resolution on connect**: connecting a non-native (but accepted) output type sets the
  input's `resolved_type`. Rendering then takes color/shape from the identity table, so the
  wire reads as one type end to end; disconnect clears it and the socket reverts to its
  idle look. `Socket::effective_type()` = `resolved_type` if set, else `type_name`.
- **Input-driven resolution only.** Outputs keep their concrete type; the only polymorphic
  output is the reroute node's `Any`.
- **No runtime adapter splicing.** The graph→pipeline compiler creates channels with the
  *resolved* type; the consuming node sets up the matching consumption path itself.

### Variadic (growing) input groups

`InputDef::variadic(max)` turns one def into a growing group: the group renders as N member
sockets ("D 1", "D 2", …) plus one trailing placeholder while members < max. Connecting to
the placeholder converts it into a member and spawns a new placeholder; disconnecting a
member removes it. Mechanics:

- `SocketId`/`Connection` are positional, so inserting/removing a socket rewrites the index
  of every stored connection above the change point (`GraphState` insert/remove helpers fix
  up `connections` atomically).
- `def_index` decouples sockets from def positions — required because controls and restore
  logic would otherwise zip sockets with defs by index.
- Variadic sockets carry no inline controls; placeholders are skipped by the compiler.

### Restore reconciliation

`restore_node` validates saved sockets structurally against the current defs (per
`def_index`: static defs 1:1, variadic defs any member count ≤ max with exactly one
placeholder unless full). A match keeps the saved sockets with per-def data refreshed
(accept lists, control presence, stale `resolved_type` cleared); any mismatch rebuilds the
sockets from the defs. Pre-variadic files (all `def_index == 0`) are upgraded positionally.

## Widget

`NodeGraphWidget` owns the graph, the runtime instances, the registry, and all interaction
state. `show(ui)` runs once per frame: build layout → allocate per-node/per-socket
responses → route input (hotkeys, menus, wire drags, selection, pan/zoom) → draw
(connections, nodes, frames, badges, minimap, panel) — a single immediate-mode pass with no
retained scene graph.

Interaction highlights:

- **Wire dragging** with live compatibility checking and snap-to-socket; a snap candidate
  previews the shape it would resolve to. Releasing on empty canvas opens a compatible-node
  search and can add/connect the selected node in one undoable gesture. Non-connectable
  nodes dim during the drag. Fast-render mode suppresses per-socket hit targets during
  heavy drags.
- **Reroute nodes** (`NodeKind::Reroute`) are model-level wire waypoints with a single
  `Any` in/out; the compiler follows wires through them. *Dissolve* removes a node and
  directly reconnects compatible in/out pairs. Double-clicking a wire inserts a reroute.
- **Frames** group nodes visually (label, color, rename-in-place, membership editing);
  dropping nodes inside/outside a frame updates membership, and empty frames are cleaned
  up automatically.
- **Node presentation**: collapse (header only) and hide-unconnected-sockets toggles;
  per-node title rename; selection (click, box select, shift-add).
- **Semantic snapshot**: `GraphState::semantic_snapshot` serializes topology, socket contracts,
  node type, mute state, node-owned processing state, and document extensions while excluding
  editor-only layout and selection. Hosts use it to avoid semantic work for visual-only edits.
- **Menus**: right-click context menu (add, cut/copy/paste, duplicate, delete, dissolve,
  frame ops, show/hide, undo/redo) and a `Shift+A` add-search popup with fuzzy matching over
  `category → name`.
- **Placement and navigation**: new, duplicated, and pasted nodes follow the pointer until
  click confirms or Escape cancels. Active drags auto-pan near viewport edges; holding Ctrl
  while dragging snaps the selection anchor to a 10-unit grid.
- **Mute/bypass**: `Node::muted` is persisted with a serde default. The model computes local
  input/output pass-through pairs from the node's declared sockets; rendering shows those
  links inside the muted node, and the product compiler follows the same pairs. Generic
  `node-graph` code never decides runtime bypass semantics.
- **Document extensions**: hosts and plugins persist namespaced state that belongs to the complete
  graph document in `GraphState`. This includes state that refers to node or socket identities,
  such as panel layout, lane ordering, selections, and subscriptions. Document extensions are part
  of save/load and whole-document undo/redo snapshots. They are deliberately excluded from node
  clipboard payloads and copied subgraphs.
- **Socket extensions**: hosts and plugins persist namespaced state that belongs locally to one
  socket on `Socket`. It travels with the containing node during copy/paste and survives node
  definition reconciliation through the socket's stable `schema_id`. A socket extension does not
  contain graph-wide node or socket references; that state belongs in a document extension.
- **Extension ownership**: generic `node-graph` serializes and preserves opaque extension values but
  never interprets, migrates, rewrites, or removes them based on their contents. Each namespace
  owner migrates only its own supported schema versions. Invalid or unsupported versions remain
  structurally unchanged as opaque JSON values until that owner can understand them. When an owner's
  document extension contains graph references, the owner removes or repairs stale references at
  its application load/mutation boundary after nodes or sockets are deleted. Unknown namespaces
  are never cleaned up by another owner. Empty extension maps are omitted for saved-file
  compatibility.
- **Tabs and contributed panels**: the built-in `Node` tab is owned by the widget; additional tabs
  are configured once per widget, and all tabs remain visible for all nodes. A `NodeDef`
  contributes opaque panels by stable panel ID and tab ID. Its
  `NodePanelPresentation` owns the title and complete UI; the widget only supplies layout,
  scrolling, state-update routing, and opaque typed panel data/action transport. Host-derived
  panel models are borrowed from a `PanelDataProvider` for one draw call and emitted actions are
  returned to the host from that same call. The widget does not retain, define, or inspect
  feature-specific panel models or actions.
- **Socket indicators**: hosts attach transient, owner-namespaced
  `SocketIndicatorPresentation` objects to any input or output `SocketId`. The widget positions
  them and delegates size and drawing to the presentation. It does not assign viewer, validation,
  or protocol semantics to an indicator.
- **Clipboard** is the system clipboard: selected nodes + their internal connections
  serialize to a JSON payload tagged `node_graph_clipboard_v1`, so copy/paste works across
  application instances. Paste remaps ids, offsets positions, selects the pasted set, and
  prunes `resolved_type` on inputs whose producer wasn't copied. Socket extensions are part of
  their nodes; document extensions are not part of this fragment format.
- **Undo/redo** snapshot the whole `GraphState` (cheap: plain data). Because sockets —
  including resolution and variadic growth — live in the model, everything undoes for free.
  Node state is synced from instances into the model before every snapshot.
- **Minimap** (toggle `Ctrl+M`): scaled-down node rectangles + viewport indicator, click/drag to
  navigate.

### Default keymap

| Key | Action |
|---|---|
| `A` / `Alt+A` | Select all / deselect all |
| `Shift+A` | Add/search |
| `Shift+D` | Duplicate, then place |
| `F2` | Rename active node |
| `.` / `Home` | Fit selection / fit graph |
| `H` / `Ctrl+H` | Collapse / hide unconnected sockets |
| `M` / `Ctrl+M` | Mute / minimap |
| `N` | Properties panel |
| `X`, Delete, Backspace | Delete |
| `Ctrl+J` | Frame selected nodes |

The contextual status hint exposes the relevant gestures for idle, selection, wire-drag,
and placement states so these accelerators are discoverable.

### Properties panel

A Blender-style N-panel docked to the right edge, rendered in **screen space** (regular
egui widgets, unaffected by graph zoom — which is what makes rich controls like channel
grids practical; inline node controls stay zoom-scaled). It shows the *active* node — the
most recently clicked/added one. The built-in *Node* panel exposes rename and type/category
info plus the def's `PanelSection`s of `PropDef`s. It is the only panel whose contents the widget
defines. Other panels come from the active node's `NodeDef`, and any configured tab can contain
multiple panels. Panel edits mutate the same node
state and run through the same `on_update` path as inline controls, so visibility,
clamping, and badges react identically. The persistent widget-level tab strip stays visible even
without an active node and toggles
the panel (`N`); the panel body floats over the graph and claims pointer input only within
its bounds. A node definition can bind its displayed title to a `StringValue` in its state. For
such nodes, inline edits, the built-in Name field, rename actions, restoration, and serialization
keep the title and state value synchronized without protocol- or node-specific widget behavior.

### External badges and statuses

The embedding application can attach its own per-node annotations, kept separate from
def-driven badges so neither clobbers the other:

- `set_node_badge(id, Option<NodeBadge>)` — compile errors, runtime warnings; takes
  precedence over the def's badge while present.
- `set_node_status(id, Option<String>)` — short live text in the node header (e.g. item
  counters while a pipeline runs).

## Persistence

The document is `GraphState`. `snapshot_value` first syncs every instance's state into the model and
returns `GraphSnapshotError` if JSON serialization fails, retaining the codec source. The host
persists the resulting value through its document service. A host loads and parses a document before
passing the resulting model to `set_graph`, which rebuilds all runtime instances through the restore
reconciliation above. New model fields use `#[serde(default)]` so older files keep loading. The
widget never opens a path itself.
