# Proposed future: Ordered obstacle-avoiding connection routing

This proposal defines a visual feature, not implemented architecture. Work is tracked by
`graph.editor.connection-routing` in [TODO.md](TODO.md) and the
[implementation plan](docs/plans/node_graph_connection_routing.md).

## Purpose

Connections avoid nodes, leave and enter sockets horizontally, and form ordered, smooth
bundles where endpoint order permits it. Routes below a node clear its bottom, not merely
its lowest socket. Wires can spread near obstacles and ports and converge in open areas.

The router first selects a valid corridor and lanes, then smooths their geometry.
Independent Bézier control-point adjustment is not the obstacle-routing algorithm.
No external routing library is required.

Priority is: preserve document topology and manual constraints; avoid node bodies;
preserve compatible lane order; maintain clearance; avoid unnecessary crossings;
retain stable routes; improve spacing and smoothness; reduce length.

## Ownership and compatibility

The implementation belongs to `node_graph`. A private directory-backed routing module
under `widget/graph` owns geometry, corridor search, ordering, validation, and transient
route state. Its `mod.rs` contains only declarations and selective re-exports. Leaves
import sibling implementations directly. Shared internal symbols use `pub(crate)`.
This feature adds no public module or external router API.

The module consumes immutable graph-space geometry and neutral connection keys, not node
definitions, protocol values, display names, runtime data, or platform services. The widget
adapts its layout and document into this input. Geometry tests need no egui context.
Native and wasm compile the same source.

`node_graph_document` retains persisted topology. Automatic guide points, bundles, history,
and curves are transient widget state. Routing does not mutate connections, create reroute
nodes, advance processing revisions, or add undo entries. No persisted schema or stable
feature-ID change is part of this proposal.

Existing reroutes are nodes with sockets and separate connections, not waypoint arrays on
one edge. Preserve that representation and its branching, editing, loading, and undo behavior.
Route each incident connection to its existing socket. Do not flatten reroute chains or
introduce persisted edge records. Reroute bodies remain obstacles with endpoint exemptions.
Frames are visual containers, not obstacles.

## Geometry, identity, and results

Routing distances use graph-space logical units, independent of pan and zoom. Initial
configurable values are horizontal clearance 20, vertical clearance 16, desired escape 30,
minimum lane spacing 6, and preferred spacing 12. Stroke, shadow, and hit tolerance remain
presentation quantities. Clearances constrain the mathematical path, not every pixel of
its screen-space shadow.

Paint connections from the exact same output socket as one group: draw all their shadows
before their colored strokes so shared runs and T junctions have no dark internal seam.
Different-source groups retain separate crossing outlines. Highlighted groups paint last,
with highlighted fills after ordinary fills within the group; hidden branches do not paint.

Inputs contain finite node rectangles, sockets with explicit left/right sides, directed
connections, configuration, and optional previous routes. Use actual layout geometry,
including collapsed and offscreen nodes. Reject non-finite geometry and invalid negative
configuration before search.

Existing connections contain only `from` and `to`. Use the socket-ID pair as a transient key
within a topology generation, never a vector index. Clear route and bundle history on
topology edits, document replacement, undo, and redo: socket-index renumbering must not reuse
another wire's history. Endpoint tuples provide deterministic tie breakers. No persisted
`EdgeId` is introduced for caching.

Internal results contain the key, classification, guide geometry, path segments, bounds,
corridor dependencies, and failure reason. Paths support lines and cubic Béziers so a
validated unsmoothed fallback is representable. Classifications distinguish ordered bundle
routes, individual routes, and unroutable connections. Failures distinguish invalid input,
blocked escape, no corridor, and exhausted work budget. Exact Rust names remain local choices.

## Port escapes and impossible layouts

Expand every node rectangle by configured clearance. Each port-to-escape segment runs
horizontally outward on its specified side. Its endpoint lies outside its own expanded
rectangle, at least horizontal clearance plus numerical safety margin away, even when
desired escape length is shorter.
The corridor joins each escape by continuing straight or turning perpendicular to it,
never by reversing along it. This also applies to extended escapes used for rounding
and retries that separate different source sockets.

Only that endpoint segment is exempt from collision with its own expanded rectangle.
It remains outside the actual node interior and is checked against every other obstacle.
The rest of the path cannot re-enter either endpoint's expanded rectangle. Do not remove
entire endpoint nodes from the obstacle set.

Overlapping nodes, covered ports, or blocked escapes can make routing impossible. Do not
silently relax body avoidance or claim success. An unroutable connection remains in the
document and uses the legacy curve with separate routing-warning markers and a hover
explanation. Wire color continues to identify its data type, with ordinary interaction emphasis;
routing failure does not override it. This diagnostic curve is outside the safe-route guarantee
but remains editable and cuttable. Non-finite endpoints instead produce a node-associated warning without
submitting invalid paint geometry. Routing recovers automatically when geometry becomes valid.

Hovering a warning marker highlights only its associated failed connections with the ordinary
port-hover emphasis, retaining data-type colors. During that hover, selected nodes do not
highlight other connections. Selection itself is unchanged and ordinary emphasis resumes
when the pointer leaves, the marker is clipped/covered, or routing recovers. The warning
tooltip continues to explain the failure.

## Ordinary and exceptional paths

The fast path handles sections whose escapes and endpoint directions permit increasing X
throughout the interior route. Ordering endpoints by X alone is insufficient. A right-side
output connected to a left-side input farther left still requires turns after swapping
endpoints. Equal-X escapes also cannot be represented by `y=f(x)`.

Backward, equal-X, and other sections without a monotonic route use an individual rectilinear
visibility search over the same obstacles. Include escape coordinates, obstacle boundaries
with numerical clearance, and a finite outer envelope around all input geometry. Connect
visible horizontal/vertical segments and allow movement in both X directions. Incoming
direction participates in search state for bend costs. Stable coordinate and endpoint
ordering resolves ties. Bound work and distinguish exhaustion from geometric failure.

The fallback preserves endpoint directions and clearance but has no bundle-order guarantee.
It also handles backward sections between existing reroute nodes. Perfect cyclic routing
and global crossing minimization are non-goals; checked individual paths or explicit
diagnostic failures are required from the first routing release.

## Bundle eligibility and crossings

Initially group only monotonic connections between the same ordered source/target node
pair, with compatible socket sides and a common corridor. Do not group by transitive
overlap of X intervals. General bundles merging across multiple nodes are deferred.

Sort distinct source sockets by Y with deterministic ties. For wires sharing an output,
sort by destination Y then endpoint identity. Their separation grows continuously from
zero in an endpoint fan-out region. Closely spaced distinct ports similarly transition
from actual spacing to configured interior spacing. Shared endpoints are not crossings.

For distinct endpoints on the two boundaries of a shared monotonic corridor, reversed
destination order requires a crossing within that corridor. This is not a claim about
arbitrary curves or edges with different X domains. Partition sorted connections into
destination-order-compatible sub-bundles, placing each in the first compatible sub-bundle
in stable order. Route incompatible groups separately. Crossings between groups are
allowed; neither sockets nor document connections are reordered.

If no common corridor fits, split into smaller contiguous groups deterministically and
retry down to individual paths. Crossings between independent groups remain allowed, but
positive-length shared tracks are reserved for connections from the exact same output
socket (node identity and socket index), never merely the same node, socket height, label,
or destination. Distinct signals do not merge into one ordinary checked wire.

After group routing, a bounded separation pass checks straight runs (including straight
cubics) and identical curved segments in stable source/destination identity order. A
conflicting later connection uses the individual visibility search with earlier signals'
parallel runs reserved. Additional coordinates offer tracks at least one lane spacing away;
perpendicular crossings remain available. Compatible-bundle peers additionally reserve
their control hulls and lane spacing, including peers later in identity order, so a repair
does not introduce a crossing inside that bundle. Same-output fan-out is exempt.

The separation pass has its own `max_work` allowance and leaves non-conflicting paths
unchanged. Each connection receives at most an equal share of the remaining work across
the remaining connections in stable order; unused work stays available for later checks.
A difficult separation retry cannot exhaust the allowance reserved for unrelated wires.
The total pass limit is unchanged. It checks node clearance and endpoint escapes on the final path; optional
smoothing is accepted only after a further shared-run check. Failed or exhausted retries
use the existing visible diagnostic fallback, not an apparently checked merged signal.
The pass also checks retained drag paths against newly routed connections before publishing
the common paint/interaction snapshot. Global crossing minimization remains a non-goal.

## Channels and capacity

Collect escape X coordinates and expanded obstacle boundaries. Sort and deduplicate with a
defined tolerance. Within each open slab, union occupied Y intervals and derive free
intervals inside a finite outer envelope. Connect adjacent intervals only through their
shared free opening at the slab boundary. Respect closed obstacle boundaries and numerical
safety margins at corner contacts.

Search once per bundle. State includes the feasible lane-band position and incoming
vertical trend needed to evaluate displacement and direction changes. Cost combines vertical
travel, bends, proximity, narrow-channel penalties, and valid prior-corridor preference.
A scalar best cost per slab interval is insufficient when entry positions differ.

For N equally spaced interior lanes reserve:

```text
required_height = 2 * bundle_margin + (N - 1) * spacing
```

Validate slab capacity AND the connecting overlap opening. Two tall channels can have an
opening too narrow for a bundle. Equivalently shrink free intervals by the bundle envelope
and test centerline feasibility. Endpoint fan-out envelopes use actual endpoint geometry
and require collision checking too.

Dynamic spacing participates in corridor selection, not a later unchecked expansion. Try
preferred spacing near obstacles/ports, then reduce toward minimum spacing when required.
Reject or split groups that cannot fit the minimum. Reserve the maximum envelope throughout
each transition before committing its corridor.

Below an obstacle, place the top lane beyond the expanded bottom plus bundle margin and
extend other lanes downward. Above it, place the bottom lane above the expanded top and
extend upward. Do not add clearance twice or center the bundle on the clearance boundary.
The entire envelope must clear other obstacles as well.

## Ordered lanes and smoothing

Within a compatible bundle's shared interior X range:

```text
y[i + 1](x) - y[i](x) >= spacing(x)
```

Endpoint regions use a continuous lower bound rising from actual port spacing, including
zero for shared outputs. Elsewhere spacing remains positive. The guarantee does not apply
between different sub-bundles.

Use a small set of guide sections with a common centerline and ordered offsets.
Corresponding cubic transitions share X control coordinates and parameterization; their Y
control coordinates retain the required gap. The Bézier convex-combination property then
preserves order for all parameter values. Represent varying required gap in that same basis
and check it coefficient by coefficient. Clamp handles to maintain monotonic X. Different
lane parameterizations cannot establish this guarantee.

Use horizontal tangents at real ports and horizontal lane entries/exits. Match endpoint
derivatives at ordinary smooth joins: parallel handles alone do not give C1 continuity.
C2 is unnecessary. Exceptional individual paths use parametric segments instead of `y=f(x)`.

Every curve requires conservative collision validation. Corridors are non-convex unions;
safe guides or control points somewhere inside a corridor are insufficient. Prove separation
from expanded rectangles using control-hull bounds and recursively subdivide ambiguous
curves. Reject candidates still ambiguous at the subdivision limit. Boundary tolerances
and endpoint exemptions are explicit. Fixed-count sampling is not a collision proof.

If smoothing fails, shorten handles with bounded retries, then retain the validated line
route. That fallback may have corners and does not claim C1 continuity, but preserves
clearance and ordered lanes. If no checked route exists, return the unroutable result.

## Rendering and interaction

Build one route snapshot from current layout. Painting and gestures consume that snapshot,
including diagnostic fallbacks. Replace endpoint-only curve reconstruction for proximity,
knife cutting, reroute insertion, and splice detection. All segments contribute to bounds
and hit testing.

Flatten curves adaptively to a screen-space error bound for interaction. Test distances
and intersections against segments, not only sampled points. Zoom changes flattening
tolerance and presentation, not corridors. Wire-drag previews share the path representation
and correct anchored socket direction but are explicitly provisional, without bundle
guarantees. Internal node pass-through decoration is not an external routed connection.

Node-on-wire insertion must not repel the wire the user intends to splice. During that
gesture, exclude only the dragged candidate from obstacles for the shared paint/interaction
snapshot and highlight the splice target; retain every other obstacle check. On release,
perform the existing topology operation and rebuild normally. This provisional exception
does not claim safety against the candidate node.

## Stability and invalidation

Cold routing is deterministic for geometry, configuration, and connection keys. Warm
routing is deterministic for those inputs plus explicit history. Hysteresis permits the
same geometry reached through different histories to retain different valid corridors.
Prefer history only while all hard constraints remain satisfied.

Initially rebuild on geometry changes. Later invalidate incident routes/bundles and routes
whose dependencies intersect an obstacle's old or new rectangle. Include size, collapse,
socket position/visibility, configuration, insertion/removal, and topology-generation changes.
Pan/zoom alone does not invalidate graph-space routes.

Bounds alone do not discover newly opened shorter corridors when an obstacle moves away.
Preserve valid routes during dragging, then schedule a bounded broader quality pass on
release. Never retain a colliding previous route just to meet a frame budget. Until fresh
routing completes, use an explicitly provisional or failed presentation rather than
claiming stale geometry is safe.

## Tests and performance

Geometry fixtures cover straight paths, above/below obstacles, narrow connecting openings,
escape exemptions, blocked escapes, overlapping nodes, backward/equal-X paths, shared-output
fan-out, inversions, deterministic splitting, reroute chains/branching, dense obstacles,
and rejected smoothing. Include invalid input, work exhaustion, boundary contacts, and
zero-length segments. Verify collisions conservatively and cubic-profile ordering
analytically; sampled overlays supplement those guarantees.

Widget tests exercise the same detours through painting, proximity, knife cuts, reroute
insertion, splice gestures, previews, collapse, variadic renumbering, undo/redo, load, and
pan/zoom. Saved topology and processing-relevant state remain unchanged by routing alone.
Test cold and history-aware determinism separately.

Debug overlays expose obstacles, escapes, intervals/openings, bundle envelopes, guides,
handles, and route classification. Visual fixtures reproduce a source-to-decoder bundle
around an intervening node using generic geometry. Names and port labels have no routing
significance.

Benchmark 100 nodes/500 connections and stress 500 nodes/2000 connections. Record cold
routing, drag-update p50/p95/p99, allocations, and complete widget frame time on documented
native and browser hardware. Initial target: routing p95 below 8 ms on the smaller fixture.
This is a measurement target, not an established result. Bound search/smoothing work;
add incremental updates after correctness and a measured baseline.

The implementation plan defines per-step verification and delivery gates.
