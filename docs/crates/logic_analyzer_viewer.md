# `logic_analyzer_viewer` Design

## Responsibility

`logic_analyzer_viewer` owns the generic egui presentation of large finite captures, growing live
captures, and retained derived output. It owns viewport interaction, row organization, cursors,
measurements, bounded visible-window sampling, sampling overlays, and renderer registration.

The widget consumes capture and derived-data query contracts. It does not parse capture formats,
build or store waveform indexes, prepare graph sources, administer repositories, or infer concrete
protocol behavior.

Implementation:

- viewer widget: [crates/widgets/logic_analyzer_viewer](../../crates/widgets/logic_analyzer_viewer)
- finite capture/index contracts: [`signal_capture` design](signal_capture.md)
- growing capture-session index: [`signal_capture_session` design](signal_capture_session.md)
- retained derived queries: [`signal_derived` design](signal_derived.md)
- concrete capture readers: [`logic_analyzer_capture_formats` design](logic_analyzer_capture_formats.md)

The supported widget API is documented at the `logic_analyzer_viewer` crate-root facade.

## Three content sources, one row list

The viewer renders three independent kinds of rows:

1. **Capture channels** — sampled on demand from a prepared generic `CaptureIndex`. Concrete graph
   source features own format-specific construction, and graph runtime owns preparation.
2. **In-memory channels** — raw `(time, level)` transition lists supplied wholesale through
   `set_channels` for host-provided data.
3. **Derived lanes** — a shared `DerivedLanes` catalog of stable payload descriptors and
   adapter-owned query handles, supplied by the UI through `RunData` and rendered through
   registered presentations.

A single `row_order: Vec<RowKey>` is the source of truth for display order across all row kinds.
The viewer reconciles it every frame before layout and hit-testing: stale rows disappear and new
rows append. Users reorder rows by dragging labels and rename them by double-clicking; rename maps
are viewer state and do not mutate the underlying capture or derived data. The header offers the
DSView Tango-based and Classic muted color profiles.

Native and wasm hosts supply the same contracts. Browser file import, embedded captures, native
files, finalized sessions, and growing sessions differ in preparation and storage ownership, not
in viewer behavior.

## Consumed capture contract

The viewer holds a `Box<dyn CaptureIndex>`. A finite `IndexSampler` and a session-owned
`GrowingCaptureIndex` implement the same metadata, sampled-window, generation, and completion
capabilities. Their summary layout, persistence, raw caches, and update algorithms are private to
their owner crates.

For the visible half-open sample range, the viewer requests a `CaptureSampledWindow` with a
pixel-derived target-point budget. The result contains one of two protocol-neutral shapes:

- exact `CaptureTransition` values for a sufficiently narrow range; or
- bounded `CaptureWaveformSegment` values classified as `Level`, `Edge`, or `Activity`.

An `Activity` segment reports truthful entry and exit levels but deliberately provides no invented
transition position. The viewer paints it as activity rather than guessing where an edge occurred.
The viewer locates predecessor and successor transitions through additional bounded sampled-window
queries, so measurements and snapping remain independent of the displayed resolution.

Source preparation publishes capture metadata and progress before the index becomes available.
The widget displays that state and repaints at a bounded cadence; it never starts an index build or
opens a source itself.

## Frame flow

Each `show()` call performs these steps:

1. Reconcile the row list against current channels and derived lanes.
2. Handle row-label editing and reordering, edge-delta measurement, cursor input, fit-to-view, pan,
   and zoom.
3. Recompute the visible `(start_sample, end_sample, target_points)` request only when the viewport
   changes, poll it, and convert a ready result into drawable channels while retaining the current
   waveform if the backing query is still pending.
4. Refresh the pulse measurement under the pointer unless an edge-delta interaction owns it.
5. Draw the header, ruler, rows, derived lanes, sampling overlays, pointer marker, measurements,
   time cursors, and color selector.
6. Request another repaint while preparation, live capture, or live derived data can change.

Capture requests are bounded by viewport width and may complete immediately or through a
host-backed proxy. Preparation and unbounded storage work remain outside the UI thread. Repaint
cadence is approximately 16 ms while opening, awaiting a sampled query, or measuring an edge;
100 ms while indexing or waiting for a sampler; 50 ms for live derived lanes; and 8 ms for a
growing capture. Otherwise egui repaints on input.

### Channel presentation model

```rust
struct LogicChannel {
    index: usize,
    name: String,
    initial: bool,
    transitions: Vec<Transition>,
    waveform: Vec<WaveformSegment>,
}

enum WaveformSegmentKind {
    Level { value: bool },
    Edge { before: bool, after: bool },
    Activity { first: bool, last: bool },
}
```

Exact results populate `transitions`; coarse results populate `waveform`. An activity segment wider
than roughly three pixels is a filled band. Narrow activity draws its first and last levels with a
center mark. Both treatments preserve uncertainty instead of displaying a false edge position.

## Collected lanes

Collected display uses two independent registries:

- `DerivedLanes` in `signal_derived` publishes stable lane keys, payload descriptors, and
  type-erased query handles owned by collection adapters.
- `WaveformPresentationRegistry` in `logic_analyzer_viewer` maps explicit group and track identities
  to those lanes and supplies protocol-neutral renderer objects. It also maps stable payload
  identities to registered singleton presentations for lanes without an explicit group.

Every visible payload belongs to an explicit or registered default group. Application composition
maps producer-owned descriptors and stable renderer keys into compound groups and renderer
objects. Row identity, labels, height, drawing, hit-testing, and snapping use group and track IDs,
not display names.

The viewer requests immutable snapshots bounded by the visible time range and a pixel-derived item
budget, then releases retained-data locks before calling renderer code. Exact and dense-activity
semantics belong to the payload query. Renderers may project a snapshot to generic level or event
transitions for measurement and event-row interaction. Cursor boundary, timeline extent, and live
status are query capabilities. Renderer and plug-in code never run while a payload store is locked,
and the viewer never branches on a concrete payload type.

Drawing receives semantic theme colors and a copied interaction context containing the bounded
window, budget, hover state, and pointer time; it does not receive `LogicAnalyzerViewer` internals.

Payload queries may publish a changing snapshot generation. The viewer retains at most two
immutable results per query identity for rendering and interaction, reuses them across egui
repaints, and coalesces live generations to the 50 ms presentation cadence. A viewport change,
query replacement, or completed-generation change refreshes immediately. Queries without a
generation remain uncached. Renderers declare whether they support interaction projection, so a
non-interactive payload does not materialize a second detail snapshot on pointer movement.

## Measurements

Measurement and edge snapping are each host-toggleable through `ui_prefs` /
`set_ui_prefs`, both enabled by default. With measurement off the viewer takes no hover pulse
measurement and starts no edge-delta measurement, and anything already on screen is dropped. With
snapping off every time position stays where the pointer put it. The host owns presenting the
toggles and persisting them across sessions.

### Pulse measurement

The hover measurement reports the high or low run under the pointer. Coarse visible segments do
not contain exact edge positions, so the viewer queries transitions around the pointer and resolves
open boundaries with predecessor and successor searches. Width, period, and duty cycle are
therefore independent of zoom level and query-window size. In-memory channels measure directly
from their transition lists.

### Edge-delta measurement

A primary click on a real transition starts an edge-delta measurement. The source remains fixed
while the endpoint follows the pointer across raw and derived rows. If the nearest transition is
within six screen pixels, the endpoint snaps to that edge; otherwise it remains free at the
pointer. Indexed raw rows use the capture index's predecessor and successor queries. Derived rows
use the renderer-provided generic transition projection.

The viewer draws a Bézier leader and a `Δt` popup. A second primary click or `Escape` stops the
interaction. While active, it owns the primary click so cursor, row-label, trigger, and pan gestures
cannot consume the stop action.

## Cursors and timeline markers

Users add vertical time cursors by double-clicking the time canvas and drag them by their flag or
line. Freed cursor numbers are reused, keeping the number-derived color stable while other cursors
come and go. Cursor drag and hover suppress panning, and cursor creation suppresses fit-to-capture
for the same event. The host persists cursor number/time pairs outside the widget and restores them
when their document becomes active.

Persisted graph timeline markers are a separate host-owned overlay. The viewer receives
protocol-neutral identity, label, and time values, draws named flags, and returns one completed move
edit after a drag. The host owns graph persistence, undo, available cursor choices, marker-to-signal
conversion, and synchronization with concrete graph nodes. Marker drags use the same row-aware edge
snapping as transient cursors; the viewer cannot create or delete persisted markers.

## Interaction summary

| Input | Effect |
|---|---|
| Drag (primary button, not on a cursor or label) | Pan the view |
| Scroll X | Pan the view |
| Scroll Y | Zoom around the pointer's sample position |
| `Home` / `F` | Fit the complete capture |
| Double-click time canvas | Add a time cursor |
| Drag a cursor flag or line | Move that cursor |
| Drag a named timeline-marker flag or line | Return a persisted-marker move to the host |
| Double-click a row label | Rename the row locally |
| Drag a row label | Reorder rows |
| Click a waveform edge | Start edge-delta measurement (when measurement is enabled) |
| `Escape` or primary click during edge-delta measurement | Stop edge-delta measurement |
| Header color selector | Switch between DSView and Classic profiles |

## Properties at a glance

| Concern | Viewer mechanism |
|---|---|
| Large capture | Request only a pixel-bounded sampled window from `CaptureIndex` |
| Full-view zoom | Render bounded level, edge, and activity summaries without opening storage |
| Deep zoom | Consume exact transitions selected by the capture owner |
| Preparation | Display metadata and progress until a prepared index becomes available |
| Growing capture | Requery the same contract while its live generation advances |
| Live derived output | Reuse generation-keyed, bounded snapshots at a controlled cadence |
| Render cost | Bound capture and lane requests by viewport width and visible time range |
| Measurement accuracy | Resolve exact neighboring transitions independently of the drawn summary |
