# `logic_analyzer_viewer` Design

Design of the waveform viewer for large DSLogic `.dsl` captures (multi-GB files) and live
pipeline output. The goal: zooming and panning stay realtime regardless of capture size,
indexing runs in the background, the UI thread never blocks on file I/O, ZIP decompression,
or raw sample scanning — and **every pixel is truthful at any timescale** (the renderer
never invents edge positions it doesn't know).

Implementation:

- egui widget: [crates/widgets/logic_analyzer_viewer](../../crates/widgets/logic_analyzer_viewer)
  (`viewer.rs`, `channel.rs`, `cursor.rs`, `draw/`, `input.rs`, `sampling.rs`)
- Index build/query engine: [crates/signal_processing/src/waveform_index/](../../crates/signal_processing/src/waveform_index)
  (`builder.rs`, `growing.rs`, `query.rs`, `storage.rs`, `reader.rs`, `types.rs`)
- Authoritative capture store: [crates/signal_processing/src/live_capture_store/](../../crates/signal_processing/src/live_capture_store)
- Capture reader / data source: [crates/logic_analyzer_processing/src/nodes/sources/dsl_file/](../../crates/logic_analyzer_processing/src/nodes/sources/dsl_file)
  (`DslCaptureReader`, `DslFileCaptureDataSource`)
- Common capture types / traits: [crates/signal_processing/src/capture/mod.rs](../../crates/signal_processing/src/capture/mod.rs)
- Derived-lane store and summary index:
  [crates/signal_processing/src/derived_data_collector/mod.rs](../../crates/signal_processing/src/derived_data_collector/mod.rs),
  [crates/signal_processing/src/derived_index.rs](../../crates/signal_processing/src/derived_index.rs)

The widget's public API is documented in
[Logic Analyzer Viewer API](logic_analyzer_viewer_api.md).

---

## Three content sources, one row list

The viewer renders three independent kinds of rows:

1. **Capture channels** — sampled on demand from a host-prepared generic `CaptureIndex`. Concrete
   graph-source builders own format-specific construction, the compiler owns preparation, and the
   widget never depends on a file format.
2. **In-memory channels** — raw `(time, level)` transition lists handed in wholesale
   (`set_channels`), used for host-provided data.
3. **Derived lanes** — a shared `DerivedLanes` catalog of stable payload descriptors and
   adapter-owned query handles that running pipeline `Viewer` nodes publish through
   (`set_derived_lanes`); rendered live beneath the channels through registered presentations.

A single `row_order: Vec<RowKey>` is the only source of truth for display order across all
row kinds, reconciled every frame (stale rows dropped, new ones appended) before any
row-position math, so hit-testing, dragging, and layout always agree. Rows are reordered by
dragging their labels and renamed via double-click (rename maps live in the viewer, keyed by
channel index / lane name — the underlying data is untouched). Two color profiles (DSView
Tango-based, Classic muted) are selectable from the header bar.

On wasm the compiler uses the same preparation and index contracts with the injected memory
repository. Host file acquisition remains unavailable until a browser adapter supplies prepared
bytes, but embedded and owned capture sources use the same indexed presentation as native sources.

---

## File Format (.dsl)

A `.dsl` file is a ZIP archive containing:

| Entry | Description |
|---|---|
| header/metadata | Sample rate, total sample count, channel list, block size, optional trigger sample |
| `L-{channel}/{block}` | Packed logic bits for one (channel, block) pair (deflate-compressed) |

Samples are divided into fixed-size **blocks** (`samples_per_block`, commonly `2^24 =
16,777,216` samples). Each `L-{channel}/{block}` ZIP entry holds one block's packed bits for
one channel.

Concrete DSL and Sigrok parsers access container entries through the processing-owned
`CaptureArchive` contract. The native adapter opens ZIP files, while parser and replay-source tests
inject in-memory archives. ZIP-specific validation remains confined to the adapter and complete
indexed-reader integration tests.

---

## Architecture

```text
concrete capture source
  │
  ├─ graph-owned CaptureIndexFactory     (opaque identity and deferred open)
  │    └─ compiler source preparation    (preload, cache, and index)
  │         ├─ background thread         (opens capture and builds/validates index)
  │         └─ concrete processing reader (DSL, Sigrok, or another registered format)
  │
  ├─ Waveform index (crates/signal_processing/src/waveform_index)
  │    ├─ IndexBuilder              — builds finite root and segment artifacts
  │    ├─ IndexReader               — reads immutable root and segment generations
  │    ├─ IndexSampler              — finite sampled_window() query handle
  │    └─ GrowingCaptureIndex       — growing sampled_window() query handle
  │
  ├─ Capture store (crates/signal_processing/src/live_capture_store)
  │    └─ CaptureStore              — committed packed chunks and finalized replay
  │
  └─ LogicAnalyzerViewer (egui)          — samples the prepared index and paints it
```

`IndexSampler` and `GrowingCaptureIndex` are the finite and growing handles of one waveform
index subsystem. They share the exact-window threshold, resolution-selection policy, capture-query
contract, per-pixel summary sampler, and presentation data types. Finite bitmap summaries and
growing tier summaries are storage backends for that shared query algorithm; neither backend emits
drawable summary spans directly. The finite handle owns an `IndexReader`, a raw
`BlockCaptureSource`, and an artifact-backed raw-block cache; the growing handle follows
committed chunks in the authoritative live store. The viewer holds either one as
`Box<dyn CaptureIndex>` — the trait seam that keeps the widget crate decoupled from storage
implementations on every target.

---

## Terminology

| Term | Meaning |
|---|---|
| Sample | A single 1-bit logic level reading at one point in time, on one channel |
| Block | The raw-capture unit; one `L-{channel}/{block}` ZIP entry, `samples_per_block` samples |
| Chunk / leaf | The serialized index payload for one (channel, block) pair: `valid_samples`, flags, and (if active) the L1/L2/L3 mipmap bitmaps |
| Directory entry | The per-(channel, block) directory record: chunk offset/length plus a duplicated top-level (L3) summary, so coarse queries never need to touch the payload |
| Segment artifact | Up to 64 channel-major leaf payloads in one bounded immutable publication |
| Root artifact | The published finite-index metadata and segment directory for one source identity |
| Raw-block artifact | A lazily populated packed source block used for exact/deep-zoom reads |

Every (channel, block) pair gets its own directory entry and chunk; the directory entry's
embedded L3 summary is what makes the coarsest zoom level cheap without another index level.

---

## Mipmap Hierarchy (per block)

Each active (non-constant) block stores three toggle levels above the raw bits. A **toggle**
bit answers "did the signal change state at least once in this group of samples?" — not which
direction. Alongside each toggle word, a same-shaped **last-value** word records the signal
level at the end of each group, so a renderer can reconstruct level without touching raw data.

```text
L1  4096 × u64   1 bit = any transition in   64 raw samples   (covers 64^2 = 4,096 samples/word)
L2    64 × u64   1 bit = any transition in 4,096 samples      (covers 64^3 = 262,144 samples/word)
L3     1 × u64   1 bit = any transition in 262,144 samples    (covers the whole 2^24-sample block)
```

`l1_last` / `l2_last` / `l3_last` are bitmaps of identical shape to their toggle counterparts,
each bit holding the signal value at the end of that group.

Memory per active block:

```text
l1_toggle = 4096 × 8 B = 32,768 B      l1_last = 4096 × 8 B = 32,768 B
l2_toggle =   64 × 8 B =    512 B      l2_last =   64 × 8 B =    512 B
l3_toggle =    1 × 8 B =      8 B      l3_last =    1 × 8 B =      8 B
total = 66,576 B ≈ 65 KiB per active block
```

**Constant blocks** (no transitions) store none of this: only `valid_samples`, `first`, and
`last` are kept, and the directory's `toggle` flag is cleared. This makes long idle regions
essentially free.

### Boundary transitions

A block's own samples may look constant while a transition actually falls exactly on the
boundary with the previous block (previous block's last sample differs from this block's
first sample). `IndexBuilder::apply_boundary_transition` detects this using the previous
block's last value and, if needed, synthesizes L1/L2/L3 toggle bits (allocating summaries for
an otherwise-constant block) so no edge is lost at block boundaries.

---

## Repository Index Format

Magic `CAPIDX07`, built by `IndexWriter` / read by `IndexReader` in
[storage.rs](../../crates/signal_processing/src/waveform_index/storage.rs):

```text
┌─────────────────────────────────────────────────────┐
│  HEADER  (96 bytes, offset 0)                        │
│    magic              [u8; 8]  = b"CAPIDX07"         │
│    version             u32     = 8                   │
│    header_size         u32     = 96                  │
│    source_revision     u64     (source file size)    │
│    total_samples       u64                            │
│    total_blocks        u64                            │
│    samples_per_block   u64                            │
│    samplerate_bits     u64  (f64::to_bits of Hz)      │
│    total_channels      u64                            │
│    blocks_per_channel  u64                            │
│    dir_offset          u64  = 96                      │
│    payload_offset      u64  = 96 + channels*blocks*40 │
│    _padding            to fill 96 bytes               │
├─────────────────────────────────────────────────────┤
│  DIRECTORY  (channels × blocks × 40 bytes)           │
│  channel-major order; one entry per (channel, block) │
│    offset     u64  (byte offset within its segment)  │
│    len        u64  (byte length of chunk)            │
│    flags      u8   bit0=toggle bit1=first bit2=last  │
│    _padding   [u8; 7]                                │
│    l3_toggle  u64  (duplicated top-level toggle word)│
│    l3_last    u64  (duplicated top-level last word)  │
└─────────────────────────────────────────────────────┘
```

Leaves use channel-major ordinals. Segment `ordinal / 64` contains up to 64 serialized leaf
payloads, and the directory entry locates each payload within that segment. A payload stores
`valid_samples`, flags, and the L1/L2/L3 arrays when active. The injected artifact repository
supplies immutable segment regions. Native regions retain their mmap backing, while memory
repositories retain owned chunks; query code does not distinguish them. `IndexReader` retains a
four-segment region cache plus its decoded-leaf cache. The compact directory is read into a
`Vec<Vec<RootDirEntry>>` at open time, so the coarsest-level query (`load_root_summary`) never
touches a segment artifact.

Validity: the header records `source_revision` (the source file's byte length) plus
`total_samples`/`total_blocks`/`samples_per_block`/`samplerate_bits`/`total_channels`. On open,
`IndexReader::is_valid` rejects a stale root so a changed capture rebuilds its index instead of
serving mismatched data. Format versions other than 8 are rejected and rebuilt. The writer
publishes every immutable segment first and publishes the root last on `finish()`, so an unfinished
generation is not discoverable.

---

## Raw Block Artifact Cache

`IndexSampler` stores a packed artifact per `(source identity, channel, block)` when an exact
query first reads that block. This cache is separate from the waveform summaries.

- A block is published only after its complete packed bytes are available.
- Reopened samplers reuse the artifact without reading or decompressing the source again.
- External one-shot `packed_block` consumers do not populate the cache implicitly.
- Repository byte regions retain mmap or owned-memory backing without a capture-sized allocation.

---

## Index Building

`IndexBuilder::build` ([builder.rs](../../crates/signal_processing/src/waveform_index/builder.rs)) runs
through the compiler-injected work executor during source preparation:

1. Enumerate every `(channel, block)` job (`total_probes × total_blocks`).
2. Submit up to 12 bounded workers through the injected executor, capped by its advertised
   parallelism and the job count. Each worker opens its own `BlockCaptureSource` reader and pulls
   jobs from a shared queue. The cap leaves host capacity for UI and other application work.
3. Each worker reads the packed block, then `build_leaf_summary` computes `first`, `last`, and
   the L1/L2/L3 toggle/last bitmaps in one pass (allocating `BlockLevels` on the heap to avoid a
   large stack frame). A block with no internal toggles yields `levels: None`.
4. Results are streamed back through a bounded channel to a single collector, which restores
   channel-major order in a small bounded reorder buffer. Each leaf is patched for boundary
   transitions against its immediate predecessor before being written.
5. `IndexWriter::write_block` appends the leaf to its bounded segment and records its directory
   entry. Each full segment is published immediately; `finish()` publishes the final segment and
   then the root header and directory.

Progress is reported as `CaptureIndexProgress { completed_roots, total_roots }` (one unit per
completed (channel, block) job).

---

## Runtime Querying — `IndexSampler`

`IndexSampler::open_data_source_with_progress` builds the index if its root is missing or invalid,
opens its root and segment artifacts, and opens a raw `BlockCaptureSource` reader for exact reads.

### `sampled_window(channels, start_sample, end_sample, target_points)`

This is the single query the viewer calls every time the visible window or viewport size
changes.

1. Clamp `[start_sample, end_sample)` to `[0, total_samples)` and compute
   `sample_step = ceil(samples / target_points)`.
2. **Exact path**: if `samples <= exact_window_sample_limit(target_points)` (at least
   `target_points × 64` samples, i.e. at least one L1 bit per rendered point, floor 4096),
   scan the raw packed bits directly (`exact_sampled_channel`) and return individual
   `CaptureTransition`s. This keeps short pulses from being widened by index summaries once the
   viewport is zoomed in close to 1:1.
3. **Indexed path**: otherwise pick the coarsest summary granularity that still resolves to
   roughly one group per rendered point:

   | `sample_step` | Group size used |
   |---|---|
   | `>= samples_per_block` | one whole block |
   | `>= 262,144` (L3) | L3 groups |
   | `>= 4,096` (L2) | L2 groups |
   | else | L1 groups |

   For each rendered point, `indexed_display_range_summary` walks the blocks overlapping that
   point's sample range and merges their `first`/`toggle`/`last` (falling back to the coarser
   directory-only `load_root_summary` when the whole block is covered or the group size is at
   least L3; otherwise `load_leaf` reads the leaf's L1/L2 bitmaps). `append_pixel_waveform`
   then turns each point's summary into one `CaptureWaveformSegment`:
   - `Activity { first, last }` if any toggle occurred in the point's range,
   - `Level { value }` if the point continues the previous level unchanged,
   - `Edge { before, after }` followed by a `Level` if the point's value differs from the
     previous point's exit value without an internal toggle.

The exact path returns `transitions` (empty `waveform`); the indexed path returns `waveform`
(empty `transitions`). `CaptureSampledWindow.sample_step` records which granularity was used.

### Raw block reads

Both the exact path and the raw-cache-backed reads for the UI's hover measurement (below) go
through `cached_packed_block`, which prefers the published raw-block artifact and falls back to
`raw_reader.read_packed_block`, publishing the complete result for subsequent queries.

---

## UI Widget — `LogicAnalyzerViewer`

Per-frame flow in `show()`:

1. `ensure_row_order()` — reconcile the row list against current channels + derived lanes.
2. Row-label input (rename double-click, drag reorder), edge-delta measurement, cursor input,
   fit-to-view (double-click / `F`), then pan/zoom input.
3. `sample_visible_window()` — recompute `(start_sample, end_sample, target_points)` for the
   current view/viewport; if unchanged since last frame, skip the query. Otherwise call
   `sampled_window` synchronously on the UI thread and convert the result into
   `LogicChannel`s. What is drawn is therefore always exactly the current view — there is no
   separate asynchronous refinement pass that could disagree with it.
4. `sample_hover_measurement()` — refresh the pulse measurement under the pointer unless an
   edge-delta measurement is active; an active edge-delta measurement instead resolves its
   endpoint from the pointer.
5. `draw()` — header, ruler, row labels, channel waveforms, derived lanes, pointer marker,
   pulse and edge-delta measurement overlays, time cursors; then the color-profile selector
   overlay.
6. Repaint scheduling: while opening (no `CaptureInfo` yet) repaint every ~16 ms; while
   indexing or waiting for the sampler, every ~100 ms; while a derived lane is live, every
   ~50 ms; and while a growing capture is active, every ~8 ms. The application may repaint at
   ~16 ms while a pipeline is active, but generation-cached derived snapshots keep those extra
   frames independent of storage-query cost. Otherwise egui's normal repaint-on-input applies.

### Channel data model

```rust
struct LogicChannel {
    index: usize,
    name: String,
    initial: bool,
    transitions: Vec<Transition>,     // exact path: individual toggles
    waveform: Vec<WaveformSegment>,   // indexed path: per-point summaries
}

enum WaveformSegmentKind {
    Level { value: bool },
    Edge { before: bool, after: bool },
    Activity { first: bool, last: bool },
}
```

`draw_channel_waveform` draws from `waveform` when present (indexed/coarse view), otherwise
from `transitions` (exact view). `Activity` segments wider than ~3 px render as a solid filled
band — a truthful "something toggled here" signal, since drawing invented edge positions would
visibly jump on refinement; narrower activity segments draw a first/last step plus a center
tick.

### Collected lanes

Collected display uses two independent registries:

- `DerivedLanes` in `signal-processing` publishes stable lane keys, payload descriptors, and
  type-erased query handles owned by their collection adapters;
- `WaveformPresentationRegistry` in `logic-analyzer-viewer` maps explicit group and track identities
  to those lanes and supplies protocol-neutral renderer objects. It also maps stable payload
  identities to registered singleton presentations for lanes without an explicit group.

Every visible payload belongs to an explicit or registered default group. The application maps
producer-owned, protocol-neutral descriptors and their stable renderer keys into compound groups
and renderer objects. Row identity, labels, height, drawing, hit-testing, and snapping use group
and track IDs rather than display names.

The viewer requests immutable snapshots bounded by the visible time range and pixel-derived item
budget, then releases retained-data locks before calling renderer code. Exact and dense activity
snapshot semantics belong to the payload query. Renderers may additionally project a snapshot to
generic level or event transitions for measurement and event-row interaction. Cursor boundary,
timeline extent, and live-status behavior are query capabilities. No renderer or plugin code runs
while a payload store is locked, and the viewer never branches on a concrete payload type. Drawing
receives semantic theme colors and a copied interaction context containing the bounded window,
budget, hover state, and pointer time; it never receives `LogicAnalyzerViewer` internals.

Payload queries optionally publish a changing snapshot generation. The viewer keeps at most two
immutable results per query identity for its bounded rendering and interaction requests, reuses
them across egui repaints, and coalesces changing live generations to the 50 ms presentation
cadence. A viewport change, query replacement, or completed-generation change refreshes
immediately. Queries without a generation remain uncached. Renderers explicitly declare whether
they provide an interaction projection, so non-interactive payloads do not materialize a second
detail snapshot merely because the pointer crosses their row.

### Pulse measurement (hover)

`sample_hover_measurement` measures the high/low run under the pointer. Because the visible
`waveform` may only carry per-point summaries at low zoom, measurement always re-queries the
index directly around the pointer (`exact_transitions_around`) rather than reusing the drawn
data, then resolves any open boundary by searching outward (`prev_transition_at_or_before`,
`next_transition_after`) so width/period/duty-cycle are exact and independent of zoom level or
query-window size. In-memory channels (no sampler) measure from their `transitions` directly.

### Edge-delta measurement

A primary click on a real transition starts an edge-delta measurement. Its source remains the
selected edge, while the endpoint follows the pointer across raw and derived rows. The endpoint
uses the target row's transition projection: when its nearest transition is within six screen
pixels it snaps to that exact edge; otherwise its time and vertical position remain free at the
pointer. Indexed raw rows resolve candidate edges through the capture index's predecessor and
successor transition queries, so snapping remains exact when the displayed waveform is a summary
band. Derived rows use their renderer-provided generic transition projection.

The viewer draws a Bézier leader from the source edge to the endpoint and a `Δt` popup. A second
primary click or `Escape` stops the interaction. While active, this interaction owns the primary
click so cursor, row-label, trigger, and pan gestures cannot consume the stop action.

### Cursors

DSView-style vertical time cursors are added by double-clicking the time canvas, dragged by their
flag or line, and numbered with freed numbers reused so a cursor's color (derived from its
number) stays stable while others come and go. Cursor drag/hover suppresses view panning and
cursor creation suppresses fit-to-capture for the same event. The host stores cursor number/time
pairs in its graph-document extension and restores them when that document becomes active.

Persisted graph timeline markers are a separate host-owned overlay. The viewer receives marker
identity, label, and time through a protocol-neutral contract, draws named orange flags on the
lower ruler row with the same dashed vertical-line rhythm as ordinary cursors, and returns one
completed move edit to the host after a drag. The host routes that edit to the marker-owning graph
node, so graph persistence and undo remain outside the widget.
Marker drags use the same row-aware edge snapping as transient cursors, but markers cannot be
created or deleted in the viewer. Concrete timeline nodes convert a marker value to a `Trigger`, a
before/at-or-after `Signal`, or an ordered `[start, end)` window `Signal`. A concrete `Cursor
Marker` node requests a numbered cursor through the generic build-context reference contract and
converts the host-supplied position into the same `TimelineMarker` runtime value. Cursor positions
are snapshotted when a run starts; moving one affects the next run. `Timeline Marker` binds its
inline name to the generic node title, so the node body and Node panel edit one synchronized value.
`Cursor Marker` exposes the host's currently available cursors as a choice list both inline and in
its settings panel; arbitrary cursor numbers cannot be entered. Its settings panel also exposes the
selected cursor's time through the same typeable nanosecond control as `Timeline Marker`. Selecting
a cursor adopts that cursor's current time, editing the time moves the selected viewer cursor, and
moving the viewer cursor refreshes the node. Generic compiler infrastructure only discovers and
routes the marker-reference binding contract; the application owns the available choices and the
bidirectional synchronization policy.

### Interaction summary

| Input | Effect |
|---|---|
| Drag (primary button, not on a cursor/label) | Pan the view |
| Scroll X | Pan the view |
| Scroll Y | Zoom, pivoted on the pointer's sample position |
| `Home` / `F` | Fit whole capture to view |
| Double-click time canvas | Add a time cursor |
| Drag a cursor flag/line | Move that cursor |
| Drag a named timeline-marker flag/line | Move the persisted graph marker |
| Double-click a row label | Rename the row |
| Drag a row label | Reorder rows |
| Click a waveform edge | Start edge-delta measurement; endpoint snaps near an edge on any row or follows freely elsewhere |
| `Esc` or primary click during edge-delta measurement | Stop edge-delta measurement |
| Header-bar combo (right) | Switch color profile (DSView / Classic) |

---

## Properties at a Glance

| Concern | Mechanism |
|---|---|
| Multi-GB file, limited RAM | Index leaves and raw blocks are bounded artifacts; native reads retain mmap pages and memory repositories retain only configured chunks |
| Zoom to full view | One block per rendered point at coarsest zoom; directory-only `l3_toggle`/`l3_last` avoids touching chunk payloads |
| Zoom to single sample | Exact path scans cached raw-block artifacts or reads one source block once the viewport is within one L1 group per point |
| Viewing during index build | Compiler source-preparation progress lets the UI show metadata and a progress bar before the prepared sampler exists |
| Constant / idle signals | No L1/L2/L3 payload stored; directory `toggle` bit cleared; reconstructed from `first`/`last` alone |
| Boundary transitions | Patched into an otherwise-constant block's summaries by `apply_boundary_transition` |
| Live decode output | Adapter-owned collected queries with bounded exact/activity snapshots, repainted while the pipeline runs |
| Render time | Bounded by viewport width (`target_points`), available index/raw data, and generation-keyed snapshot reuse |
| Measurement accuracy | Always resolved via direct index queries, independent of the zoom level currently drawn |
