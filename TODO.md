# TODO

## Logic-analyzer viewer

- Add global and per-lane height zoom, using modifier + scroll-wheel input.
- Support displaying multiple capture sources in the logic-analyzer viewer.
- Let the viewer select which source is visible while the one-source display restriction
  remains.
- Add time offsets and alignment controls for sources, including a clear shared time-base
  model.
- Display live-source snapshots in the viewer through the same `CaptureDataSource` boundary
  used by file captures.
- Make sampling-point overlays passive viewer data. Move clock-edge selection, qualifier
  evaluation, and sampled-value lookup out of `logic_analyzer_viewer` into the owning concrete
  runtime node or neutral processing infrastructure. Pass explicit, generic sampling-point
  records and presentation metadata to the viewer so an overlay reflects produced data rather
  than the viewer interpreting raw channels before the node has run.

## Capture sources

### Consolidate wasm stand-ins behind processing platform facades

- Make `logic_analyzer_graph_nodes` compile the same concrete node definitions and runtime builders on
  native and wasm. It must describe node state, ports, and presentation contracts without knowing
  that a wasm runtime is synthetic or that a native runtime uses USB/filesystem resources.
- Move selection of real versus synthetic source and sink implementations into whole-file
  platform facades owned by `logic_analyzer_processing`. The U3Pro16 facade selects the USB-backed
  implementation natively and a synthetic implementation on wasm; file-source facades select
  native readers or deterministic in-memory captures; writer facades select filesystem writers or
  discard sinks.
- Prefer a platform-neutral factory or wrapper with one constructor/configuration surface. Use a
  type re-export alias only where the native and wasm implementations genuinely satisfy the same
  API; do not force hardware-only control methods onto synthetic implementations merely to make an
  alias compile.
- Pass synthetic capture presentation and runtime capabilities back through explicit processing
  metadata/contracts. Remove `builder_wasm.rs`, synthetic-presentation helpers, and target-specific
  builder registration from `logic_analyzer_graph_nodes` once the processing facade owns those choices.
- Keep target selection in one processing `platform` boundary per capability and add native/wasm
  catalog, port-schema, state-option, and lowering-parity tests.

### Live-capture extended workflows

- Introduce repeated and segmented acquisition with frame identity, per-frame origin and trigger
  metadata, bounded storage, replay, and viewer navigation.
- Add live search and measurements over committed raw/derived prefixes with explicit coverage and
  lag.
- Add host capabilities for capture lifecycle, integrity, storage, and sleep inhibition without
  platform conditionals in consumers.
- Expose the same validated coordinator commands and outcomes through a UI-independent automation
  service.
- Add external trigger/clock contracts and shared-timeline alignment after multi-source viewer
  support is defined.
- Make file and live sources first-class capture providers, rather than having the app select
  source types explicitly.
- Persist/reload live-capture snapshots where appropriate so they can be indexed and revisited.
- Extend Sigrok support beyond v2 digital `logic-*` data (analog channels and newer format versions).

## Indexed derived data

- Run the ignored release-mode writer differential and golden graph tests against the complete
  reference capture; record output sizes and hashes and ensure temporary artifacts are contained.
- Add read-only derived-cache inventory/usage reporting to complement the existing clear-cache
  commands. Active mapped entries must remain pinned and visible as retained.
- Profile egui update, indexed sampling, lane-lock duration, repaint cadence, and input latency
  while decoding a complete capture; add focused regressions for any reproduced stall.
- Optionally profile the indexed-store append pipeline toward the sub-50-second full-cache stretch
  target. Optimize only measured builder/encode/write phases while preserving fingerprints,
  bounded RSS, query latency, and cancellation.
- Audit native `DerivedLaneData::Annotations` paths after plugin/wasm compatibility is confirmed;
  remove only duplicate native retention while preserving wasm, explicit in-memory mode, and
  storage-failure fallback.

## Graph and runtime

### Node-graph widget

- Revisit the `set_panel_data` attachment API. Client code has the node and panel IDs and should
  remain the authoritative owner of panel state; `NodeGraphWidget` must not become a general-purpose
  or persistent client-data store. Consider a draw-scoped `PanelDataProvider`/action handler so the
  widget can borrow panel models without retaining them. Preserve an explicit attachment mechanism
  only where transient, widget-lifetime data is genuinely useful, and document its ownership,
  replacement, cleanup, and non-persistence semantics.
- Revisit ownership of persistent graph and socket `extensions`. Although opaque, namespaced JSON
  lets hosts and plugins preserve saved-document metadata without coupling generic graph code to
  its meaning, it also makes `node_graph::GraphState` responsible for storing application data such
  as panel layout, viewer lane order, sampling overlays, viewer selections, and payload
  subscriptions. Decide whether this belongs in the generic graph model or in a host-owned saved
  document/envelope surrounding the graph. Include unknown-plugin round-tripping, migration,
  copy/paste and subgraph behavior, socket metadata, and eventual extraction of `node_graph` as a
  standalone widget in that decision; do not move the data until the ownership contract is clear.

### Multi-source timeline

- Define how several source clocks and trigger positions map onto the shared viewer timeline.
- Add graph-level source grouping/alignment metadata and preserve it in saved graphs.

### Node-graph extraction

- Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
