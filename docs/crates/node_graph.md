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

`NodeDef::add_menu_category` supplies protocol-neutral `AddMenuCategory` metadata. Its path
creates nested menus and its root-order value positions the top-level category while preserving
registry order between equal values. Dynamic `NodeTemplate` entries carry the same metadata, so
the widget never derives ordering from category labels.

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
member removes it. Dropping a wire on a member that is *already* occupied inserts it there
(`GraphState::insert_variadic_connection`): the member and every one after it shifts down a
place carrying its own link, and the group grows by one — replacing only when the group is
already at its max. Mechanics:

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

The layout owns a transient map from connection endpoint pairs to `WirePath` geometry.
Each path holds exact line/cubic segments, conservative control-hull bounds, and an
adaptive line approximation for interaction. Painting, proximity checks, knife cutting,
reroute insertion, and node splicing consume that geometry; they do not reconstruct
curves from endpoints. Layout rebuilds also rebuild the map, so socket renumbering and
document replacement cannot reuse stale connection identities.

The private `widget/graph/routing` owner contains geometry and path painting without
document or node-definition knowledge. Connection styling belongs to the enclosing
widget's `connection_paint` owner. Checked external connections use rectilinear paths with
optional validated curves on individual connections and ordered bundles.
Failed routes and drag previews use output-first cubic handles with a 50-screen-pixel
minimum horizontal span; their layout-space representation scales that minimum with zoom.
Interaction subdivision targets a half-screen-pixel error, with
a finite depth limit for pathological geometry. Queries test the resulting segments,
including collinear overlap, rather than isolated samples. This approximation is not an
obstacle-collision proof. Preview wires use the same geometry representation; muted-node
internal dashed links remain separate decoration. Paths are not persisted or added to
undo history.

Visible connections from the exact same output socket form one paint group. All outlines
in that group are drawn before its colored strokes, so shared runs and T junctions have
no internal shadow seam. Different output sockets remain separate groups, including equal
port indices on different nodes, and keep their crossing outlines. Groups containing a
highlighted branch paint after ordinary groups; within a group, highlighted fills paint
last without changing other branches' emphasis. Hidden branches contribute no outline or
fill. Painting uses the unchanged path snapshot used by hit testing and editing gestures.

The private individual router accepts node rectangles and explicit left/right port geometry.
Its layout adapter includes offscreen nodes, excludes frame rectangles, and assigns temporary
obstacle indices in sorted node-ID order. Every layout generation computes checked routes
without changing the document. The adapter partitions connections by ordered node pair, then
sorts source sockets by height and socket index. Shared outputs use destination height and
socket index as their secondary order. First-compatible placement partitions destination
inversions into separate candidate groups. Equal heights, including signed zero, use socket
keys rather than container iteration order. Non-finite, backward, and equal-X candidates
remain singletons. A separate bounded comparison allowance limits partitioning; exhaustion
also leaves remaining candidates as singletons. Candidate membership alone does not assert a
shared corridor: the bundle router checks capacity and final geometry before accepting a group.

The bundle router searches horizontal lane bands near the endpoint midpoint and above/below
expanded obstacle boundaries. Finite band coordinates are sorted numerically and deduplicated
before proximity ordering, avoiding repeated distance comparisons for aligned obstacle rows.
Proximity uses `f64` distance and a total coordinate tie-breaker; in-place unstable sorting
preserves the exact candidate order and the negative-zero representative. Candidate generation
is charged before deduplication, so repeated coordinates do not reduce the work-budget charge.
Interior lanes have a fixed minimum spacing of eight layout units. Staggered rectilinear
endpoint fans transition from actual socket spacing to interior
spacing. The complete lane band and both connecting fan envelopes must avoid every expanded
body. Final segments receive the same analytic collision checks as individual routes; only
the outward port escapes have their own-node exemptions. Pairwise analytic checks reject
intersections within a bundle, except the horizontal prefix/suffix of an explicitly shared
socket. Coincident distinct sockets do not receive that exemption. Shared-output separation
starts at zero; no minimum-spacing guarantee applies inside endpoint fans.

When a single band does not fit, the router searches a monotonic visibility lattice for a
shared multi-turn reference path. Endpoint band pairs are tried in stable proximity order
after validating their fan openings. An asymmetric swept footprint reserves the full lane
height below the reference and the full turn displacement on either side in X. Obstacle
inflation by the reflected footprint checks both slab capacity and connecting openings,
including the space required to stagger vertical runs. Lane offsets turn in opposite X
directions for upward and downward runs. Collinear reference steps are coalesced; folded
offsets, unrepresentable spacing, and intersecting final paths are rejected. The conservative
footprint can reject a physically feasible layout; search does not claim completeness or a
globally optimal bundle arrangement.

A failed bundle attempt splits into contiguous halves in stable order and retries
down to individual visibility routes. Crossings between separately routed groups are allowed.
All band candidates, collision checks, retries, and individual searches share the routing
work budget. The private `routing/corridor` owner provides rectangle validation, port escapes,
and visibility search to the individual and bundle routers; neither routes through the other.

The subsequent source-separation pass has its own `max_work` limit. Each connection's
overlap proof and retry receive at most an equal share of the remaining work across the
remaining connections in stable identity order. Unused work returns to the pass budget.
An expensive retry cannot spend the work reserved for checking unrelated paths later in
the pass, including retained drag paths. Exhaustion remains local to that connection;
non-conflicting paths retain their checked geometry, and the total pass limit is unchanged.

Before exact segment validation, a rectilinear bundle builds one closed envelope containing
all its lanes, fan-outs, and endpoint escapes. A budgeted broad-phase scan retains every
expanded obstacle touching that envelope, with original obstacle indexes for exemptions.
Exact segment checks use that subset and reject any segment outside the envelope. This
eliminates repeated scans of irrelevant bodies without changing closed-boundary collision
rules, endpoint exemptions, candidate ordering, or lane-separation proof. Obstacle expansion,
escape validation, and corridor search still consider the complete input geometry.

Each checked rebuild shares one routing work budget across connections in stable
pair/group/member order. The widget retains one transient routing snapshot, keyed by
all node rectangles, the ordered connection list and its actual socket positions,
provisional exclusion, the complete routing configuration, and zoom. Identical inputs
reuse both paths and failure classifications; pan and screen origin are not routing
inputs. Outside geometry gestures, any mismatch rebuilds the whole snapshot.

During node/frame dragging or node placement, unchanged node pairs retain checked paths
after dependency revalidation. Node insertion/removal, connection identity/order changes,
configuration, zoom, and provisional-exclusion changes reset history. Changed endpoint
bodies or actual socket positions invalidate the entire node pair. For other changed
bodies, old and new expanded bounds identify potentially affected paths; exact rectilinear
checks and conservative cubic subdivision prove clearance against the current obstacles.
Unchanged obstacles retain their previous proof. Changed non-endpoint obstacles receive
no endpoint escape exemption. Invalid geometry or inconclusive/exhausted proof rejects reuse.

Retention is atomic across every connection of a node pair, including previously split
bundles, so old ordered members never mix with independently rerouted members. Failed
paths are not history candidates. Optional history validation has its own bounded
`max_history_work` allowance; rejected pairs use the normal shared search/quality budgets.
Valid prior corridors remain stable during dragging even when shorter corridors open.
The first layout after the gesture ends performs a full bounded rebuild, including when
the geometry matches the final drag snapshot. This also applies to cancellation. Cold
routing is deterministic from inputs; drag routing is deterministic from inputs and history.

Paths share immutable curve, interaction, and bounds data through `Arc`, so cached and
earlier live layouts remain independent without copying flattened geometry. Zoom changes
rebuild the screen-tolerance approximation and diagnostic fallback geometry. Document
replacement explicitly discards the snapshot; routes never enter saved graphs or undo
records. Work-limit fallbacks remain visibly classified on cache hits rather than being
promoted to checked routes, and are retried after routing inputs change.

The router validates finite geometry and configuration, expands node bodies, and creates
horizontal escape segments. Only each escape's own expanded body is exempt from its collision
check; every other body remains an obstacle. Escape endpoints lie beyond their expanded body
even when the requested escape is shorter than the clearance.
Individual and source-separation searches constrain their first and last transitions to
join these escapes without a reversal. The same constraint applies when optional quality
work lengthens the escapes, so a wire cannot double back into a protruding endpoint spur.

Search uses a finite visibility lattice at escape coordinates and just outside obstacle
boundaries. Columns partition the plane into slabs; search retains Y position and incoming
direction rather than collapsing an entire free interval to one cost. Forward routes first
use increasing-X transitions; a failed monotonic search retries with both horizontal directions.
Backward and equal-X connections use that bidirectional search directly. Candidate edges and
final line segments use analytic closed-rectangle intersection checks, including corner contact.
The outermost coordinates provide a finite envelope outside all obstacles. Coordinate counts,
state allocation, search, and validation consume explicit limits; invalid geometry, blocked
escapes, no corridor, and exhausted work are distinct errors. Successful results use the same
line/cubic path contract as painting.

Individual routes have optional quality work with a separate 50,000-unit frame budget.
When a checked route turns at an escape endpoint, a budget-isolated search tries longer
straight escapes, reserving two corner radii beyond the larger of the configured escape
and horizontal clearance plus safety. Failed reservation retains the original checked route.
Corner rounding preserves each mandatory straight escape, coalesces collinear interior
steps, and tries up to six decreasing radii at each eligible corner. Endpoint transitions
can trim only the reserved extension, never the mandatory escape.
Accepted cubic handles align with the adjoining line tangents. Each curve must avoid all
expanded bodies, with no endpoint-node exemption. Unproved corners stay sharp; exhausted
rounding work restores its checked input path without spending the routing-search budget
or marking a safe route as failed. This pass does not assert derivative-matched joins or
bundle ordering and does not modify bundle paths.

Bundle corridor preferences share the optional quality budget. A checked minimum-spacing
route is retained first. Before committing the displayed corridor, optional searches try
12-unit preferred spacing with a full corner-radius clearance reservation, minimum spacing
with that reservation, then preferred spacing with ordinary clearance. Failure or exhausted
quality work retains the minimum-spacing route without consuming additional safety-search
work. The selected spacing determines the endpoint-fan width; smoothing validates against
ordinary clearance, allowing curves to use the reserved room. This selected spacing is
the interior lower bound for subsequent local widening.

Bundle smoothing is a separate, group-atomic quality pass sharing the frame's smoothing
budget. It merges overlapping transition windows around vertical runs and places common
X knots at their boundaries and at the two endpoint-fan boundaries. All lanes use identical,
strictly increasing X control points on each cubic section. Their ordered Y control
coefficients prove whole-curve ordering via nonnegative Bernstein weights; equal coefficients
are allowed only for explicit shared-socket prefixes/suffixes. Interior coefficients preserve
the configured minimum lane spacing. Horizontal Y handles give zero endpoint slope with
respect to X, matching derivatives at section joins and the unchanged horizontal escapes.

Every candidate cubic must pass conservative obstacle validation. Up to twelve decreasing
transition widths are tried. Invalid ordering, unrepresentable controls, exhausted quality
work, or unproved clearance restores the entire checked group; smoothed and unsmoothed lanes
are never mixed within one bundle. The pass can shorten unnecessary excursions.

Validated cubic bundles can widen locally toward preferred spacing. Common knots near
expanded obstacle boundaries and at interior interval midpoints expose changes in available
room. At each interior knot, up to six decreasing widening amounts try centered expansion,
then expansion anchored at either outer lane. This permits asymmetric centerline shifts.
Both adjoining sections of every lane must pass collision proof before the knot is committed;
actual f32 coefficient gaps must not decrease. Common X controls and zero Y(X) derivatives
remain unchanged, preserving whole-curve order and smooth joins. Endpoint fans and their
boundary spacing stay fixed. Narrow sections retain their checked lower-bound spacing, and
groups that cannot fit even at minimum spacing are split by the checked router. Exhausted
local-widening work retains the fully validated cubic bundle, including earlier certified
knot changes. This bounded local search does not claim globally optimal spacing.

Cubic collision validation is separate from interaction flattening. It uses convex-hull
rectangle separation and bounded de Casteljau subdivision with outward-rounded f64 interval
control points. Strict separation proves clearance; contact or unresolved overlap at the
depth bound is rejected. Painting and gestures share the accepted line/cubic path, whose
interaction approximation uses a half-screen-pixel tolerance at the current zoom.

Failed connections use editable fallback curves that retain the connection's resolved
data-type color and ordinary selection/interaction emphasis. Separate warning markers and
hover explanations report routing failures without recoloring wires. A canvas summary also
reports failures whose socket geometry cannot be drawn; non-finite endpoint geometry never
creates a fallback path. Layout edits automatically retry
routing. The empty-canvas context menu opens Routing diagnostics, with independent overlays
for expanded obstacles, port escapes, and route results. Diagnostic settings are transient
widget state and do not alter the document or its processing revision.

Hovering a routing-warning marker highlights the failed connections represented by that
marker using the same brighter, thicker data-type-colored stroke as port hover. Other wires
temporarily ignore selected-node emphasis; leaving the marker restores ordinary highlighting
without changing selection. Marker tooltips remain available. Hidden carried links stay
hidden, and clipped markers or markers covered by floating panels do not override emphasis.

An unconnected node being dragged or singly placed is temporarily excluded from routing
obstacles so it can target a wire for splicing. The paint and interaction snapshot share this
provisional geometry and highlight the splice candidate. Release checks the current node
position, applies the existing topology operation, and returns to ordinary obstacle checking.
Existing reroute nodes and their branching connections retain their saved representation.

The wire-splice owner checks connectivity before preparing a drop-specific routing layout.
Connected nodes cannot splice, so drag and placement confirmation leave their routing history
untouched until the ordinary post-gesture quality rebuild. Eligible unconnected nodes still
prepare candidate-excluded geometry at their final position, including center-targeted drops
without a pointer. Frame membership and the final full routing rebuild remain independent
of this insertion check.

Node hit targets are initially registered for input, then raised in paint order so a
front node covers controls belonging to nodes behind it. Targets outside the drawing clip
keep their initial registration but skip the redundant z-order update. Visibility is
tested per target, not per node body, so protruding socket hit areas remain interactive
at viewport edges. Routing continues to include all offscreen obstacle geometry.

Below 60% zoom, fully visible isolated nodes with unchanged input/drawing target geometry
retain all registrations and focus refreshes but omit redundant z-order list moves. The
move plan preserves the desired relative order of every pair of targets that can share a
direct or near hit, using actual initial socket ranks and drawing order. Overlapping or
clipped bounds, transformed layers, invalid interaction radii, and changes to target geometry,
target identities, or minimap visibility use full raising. Planning is skipped during fast
rendering and on release frames whose
initial allocation contains only the canvas; those node targets are new insertions.

Each layout groups socket hit-target identities by node, copying their relative order from
the flat hit-rectangle map. Raising a node visits only its own targets, preserving the
flat-map overlap winner without rescanning every other socket. The index is rebuilt with
each snapshot, reserves capacity from that node's socket counts, and reads rectangles from
the same snapshot; it does not cache document or geometry state between frames.

Socket layout queries connectivity only when a socket would otherwise be hidden;
already-visible sockets do not scan the connection list. Hidden connected sockets remain
laid out. Indicator painting resolves connectivity only when that socket has decorations,
preserving connected/unconnected placement and owner ordering without storing a cached
connectivity state.

Interaction highlights:

- **Wire dragging** with live compatibility checking and snap-to-socket; a snap candidate
  previews the shape it would resolve to. Releasing on empty canvas opens a compatible-node
  search and can add/connect the selected node in one undoable gesture. Non-connectable
  nodes dim during the drag. Fast-render mode suppresses per-socket hit targets during
  heavy drags.
- **Link moves.** An input holds one link, so dragging a connected input detaches it and
  drags the free end, anchored at its source output. An output may feed many inputs, so a
  plain drag from it adds another link and the `move_link` binding (`Ctrl` by default)
  picks an existing one up instead, anchored at the input it keeps — the nearest
  destination when the output feeds several. A carried link stays in the document and is
  only hidden until the drag lands, where connecting to the anchored input displaces it in
  one step. Rewiring never removes a link first and re-adds it against a saved `SocketId`:
  removing it reverts the input socket, and a collapsing variadic member renumbers every
  socket after it, so the saved id would address a different socket — the same ordering
  rule governs reroute insertion and node-on-wire splicing. On a reroute point, whose body is barely
  wider than the two socket hit areas flanking it, the modifier picks the link up from
  anywhere on the point while a plain drag still moves the point itself. A hovered socket
  reports the `node_graph.socket` binding context, which is how the host status bar
  advertises both the drag and its modifier.
- **Reroute nodes** (`NodeKind::Reroute`) are model-level wire waypoints with a single
  `Any` in/out; the compiler follows wires through them. *Dissolve* removes a node and
  directly reconnects compatible in/out pairs. Double-clicking a wire inserts a reroute,
  as does the modifier-qualified `insert_reroute` click bound alongside it (Command-click
  as shipped).
  A point is narrower than one socket hit area, so each of its sockets keeps the outer
  quarter of the point plus all of its reach outside it, leaving the middle half as the
  drag handle that moves the point.
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
and placement states so these accelerators are discoverable, and names the socket gestures
while the pointer is over a socket.

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
