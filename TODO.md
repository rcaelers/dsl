# TODO

Task IDs start with their ownership category and remain stable when task wording changes.

## User-visible features

### Logic-analyzer viewer

- [viewer.multiple-sources] Support displaying multiple capture sources in the logic-analyzer viewer.
- [viewer.source-selection] Let the viewer select which source is visible while the one-source display restriction
  remains.
- [viewer.source-alignment] Add time offsets and alignment controls for sources, including a clear shared time-base
  model.
- [viewer.live-snapshots] Display live-source snapshots in the viewer through the same `CaptureDataSource` boundary
  used by file captures.

### Capture sources

- [capture.live.segmented-acquisition] Introduce repeated and segmented acquisition with frame identity, per-frame origin and trigger
  metadata, bounded storage, replay, and viewer navigation.
- [capture.live.partial-analysis] Add live search and measurements over committed raw/derived prefixes with explicit coverage and
  lag.
- [capture.live.automation-service] Expose the same validated coordinator commands and outcomes through a UI-independent automation
  service.
- [capture.live.external-timing] Add external trigger/clock contracts and shared-timeline alignment after multi-source viewer
  support is defined.
- [capture.live.snapshot-persistence] Persist/reload live-capture snapshots where appropriate so they can be indexed and revisited.
- [capture.sigrok.extended-formats] Extend Sigrok support beyond v2 digital `logic-*` data (analog channels and newer format versions).

### Indexed derived data

- [derived.cache.inventory] Add read-only derived-cache inventory/usage reporting to complement the existing clear-cache
  commands. Active mapped entries must remain pinned and visible as retained.

### Node graph editor

- [graph.editor.socket-renaming] Add generic instance-local socket renaming. Node definitions explicitly mark which input and
  output sockets are renameable; sockets without that capability remain definition-owned. Preserve stable schema IDs and
  runtime port contracts independently from display names, persist user overrides in saved graphs, and provide a way to reset
  a renamed socket to its definition-provided label.

### Graph nodes

- [graph.nodes.measurement-statistics] Add generic measurement and statistics nodes for frequency, duty cycle, pulse width,
  inter-event timing, counts, and histograms.
- [graph.nodes.script-nodes] Add custom script nodes, initially backed by Python, as a plugin/runtime capability with an
  explicit manifest for input/output payload kinds, state schema, parameter defaults, and
  presentation metadata. Run scripts behind a versioned worker boundary with cancellation,
  diagnostics, resource limits, deterministic test fixtures, and an unavailable-platform error;
  do not let scripts access widget state or make the compiler infer contracts from Python code.

### Multi-source timeline

- [graph.timeline.shared-clock-model] Define how several source clocks and trigger positions map onto the shared viewer timeline.
- [graph.timeline.source-grouping] Add graph-level source grouping/alignment metadata and preserve it in saved graphs.

## Refactorings

### Logic-analyzer viewer

- [viewer.passive-sampling-overlays] Make sampling-point overlays passive viewer data. Move clock-edge selection, qualifier
  evaluation, and sampled-value lookup out of `logic_analyzer_viewer` into the owning concrete
  runtime node or neutral processing infrastructure. Pass explicit, generic sampling-point
  records and presentation metadata to the viewer so an overlay reflects produced data rather
  than the viewer interpreting raw channels before the node has run.

### Capture platform boundaries

- [capture.platform.node-parity] Make `logic_analyzer_graph_nodes` compile the same concrete node definitions and runtime builders on
  native and wasm. It must describe node state, ports, and presentation contracts without knowing
  that a wasm runtime is synthetic or that a native runtime uses USB/filesystem resources.
- [capture.platform.processing-facades] Move selection of real versus synthetic source and sink implementations into whole-file
  platform facades owned by `logic_analyzer_processing`. The U3Pro16 facade selects the USB-backed
  implementation natively and a synthetic implementation on wasm; file-source facades select
  native readers or deterministic in-memory captures; writer facades select filesystem writers or
  discard sinks.
- [capture.platform.neutral-factories] Prefer a platform-neutral factory or wrapper with one constructor/configuration surface. Use a
  type re-export alias only where the native and wasm implementations genuinely satisfy the same
  API; do not force hardware-only control methods onto synthetic implementations merely to make an
  alias compile.
- [capture.platform.presentation-contracts] Pass synthetic capture presentation and runtime capabilities back through explicit processing
  metadata/contracts. Remove `builder_wasm.rs`, synthetic-presentation helpers, and target-specific
  builder registration from `logic_analyzer_graph_nodes` once the processing facade owns those choices.
- [capture.platform.parity-tests] Keep target selection in one processing `platform` boundary per capability and add native/wasm
  catalog, port-schema, state-option, and lowering-parity tests.
- [capture.live.host-capabilities] Add host capabilities for capture lifecycle, integrity, storage, and sleep inhibition without
  platform conditionals in consumers.
- [capture.live.provider-unification] Make file and live sources first-class capture providers, rather than having the app select
  source types explicitly.

### Indexed derived data

- [derived.validation.reference-capture] Run the ignored release-mode writer differential and golden graph tests against the complete
  reference capture; record output sizes and hashes and ensure temporary artifacts are contained.
- [derived.performance.ui-latency] Profile egui update, indexed sampling, lane-lock duration, repaint cadence, and input latency
  while decoding a complete capture; add focused regressions for any reproduced stall.
- [derived.performance.append-pipeline] Optionally profile the indexed-store append pipeline toward the sub-50-second full-cache stretch
  target. Optimize only measured builder/encode/write phases while preserving fingerprints,
  bounded RSS, query latency, and cancellation.
- [derived.annotations.native-retention] Audit native `DerivedLaneData::Annotations` paths after plugin/wasm compatibility is confirmed;
  remove only duplicate native retention while preserving wasm, explicit in-memory mode, and
  storage-failure fallback.

### Node-graph ownership

- [graph.widget.panel-data-ownership] Revisit the `set_panel_data` attachment API. Client code has the node and panel IDs and should
  remain the authoritative owner of panel state; `NodeGraphWidget` must not become a general-purpose
  or persistent client-data store. Consider a draw-scoped `PanelDataProvider`/action handler so the
  widget can borrow panel models without retaining them. Preserve an explicit attachment mechanism
  only where transient, widget-lifetime data is genuinely useful, and document its ownership,
  replacement, cleanup, and non-persistence semantics.
### Node-graph extraction

- [graph.extraction.standalone-crate] Prepare `node-graph` for an eventual separate repository: replace workspace-inherited
  package/dependency metadata when extraction is scheduled, move its documentation and
  examples with the crate, add standalone CI, and make native file-dialog integration an
  optional feature or host capability.
