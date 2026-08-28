# LogicConduit

A desktop application for capturing, decoding, and analyzing digital signals. You draw
a decode pipeline as a node graph — sources, protocol decoders, logic, file writers, a
waveform viewer — press **Run**, and watch the results appear live in the built-in logic
analyzer view. Graphs run on a streaming, thread-per-node engine and can be edited while
they run.

## Features

- **Node-graph editor** (Blender-style): searchable add menu, frames, reroutes, undo/redo,
  copy/paste, a properties panel, and per-node validation badges.
- **Logic analyzer view** for multi-GB `.dsl` captures: realtime pan/zoom at any scale
  (background indexing, never blocks the UI), time cursors, exact pulse measurement,
  and live lanes showing decoded output while a pipeline runs.
- **Protocol decoders**: SPI, UART, parallel/binary bus (SDR and DDR); plus matchers,
  gates, flip-flops, counters, formatters, and file writers for building
  trigger-and-capture logic out of nodes.
- **Live editing**: attach a new matcher or viewer lane, tweak a pattern, or remove a
  branch while the pipeline is running — the engine applies the smallest possible change.
- **Sources**: `.dsl` capture file replay and live DSLogic U3Pro16 USB capture.

## Quick start

```bash
# Build and start the editor (release mode recommended)
cargo run --release --bin logic-conduit

# Or open a graph directly
cargo run --release --bin logic-conduit -- graphs/spi_controlled_decode.json
```

The editor starts with an empty graph. Use **File ▸ Load** (`⌘O`/`Ctrl+O`) to open a saved
graph — example graphs are in [graphs/](graphs) — then press **▶ Run** in the toolbar.
If the graph contains a *DSL File Source* node, its capture file opens automatically in
the waveform view above the graph.

## The editor

The window is split by a draggable divider: the **logic analyzer view** on top, the
**node graph** below, with the Run/Stop toolbar between them.

### Editing the graph

| Action | How |
|---|---|
| Add a node | `A` or right-click ▸ Add / Search |
| Connect | Drag from a socket to a compatible socket (incompatible ones won't snap) |
| Disconnect | Drag a wire off its input |
| Select | Click; box-drag; shift-click to extend |
| Move | Drag node headers |
| Pan / zoom | Drag empty canvas / scroll |
| Cut, copy, paste | `⌘X` / `⌘C` / `⌘V` (works across app instances) |
| Duplicate | `⇧D` |
| Delete | `Delete`, `Backspace`, or `X` |
| Undo / redo | `⌘Z` / `⇧⌘Z` |
| Properties panel | `N` (or the tab strip on the right edge) — settings of the active node |
| Minimap | `M` |
| Frames (group nodes) | select ▸ `⌘J`; rename/recolor via right-click |
| Hide unconnected sockets | `⌘H`; collapse a node via right-click |
| Reroute wires | Add a *Reroute* node as a wire waypoint |

(macOS `⌘` = Ctrl on Linux/Windows.)

Socket shapes and colors tell you what fits where: **circles** are continuous signals
(logic levels, counts, text — anything with a value at every instant), **diamonds** are
events (decoded words, triggers), **squares** are fixed settings. Colors group payload
kinds (green = logic, orange = words, amber = triggers, blue = numbers, rose = text).
Nodes that can't compile show a badge explaining why.

### Running

**▶ Run** compiles the graph and starts it; the toolbar shows *Live* while data flows and
node headers show live item counts. You can keep editing while it runs — most changes
(new branches, removed branches, pattern/template tweaks) apply within half a second
without disturbing the rest of the pipeline; changes that need a full restart say so in
the toolbar. **⏹ Stop** winds the run down; output files are flushed and closed.

### The logic analyzer view

| Action | How |
|---|---|
| Pan / zoom | Drag, scroll horizontally / scroll vertically (zooms around the pointer) |
| Fit whole capture | Double-click or `F` |
| Time cursors | Double-click the ruler to add; drag a cursor's flag to move it |
| Measure a pulse | Hover it — width, period, and duty cycle, exact at any zoom |
| Rename a row | Double-click its label |
| Reorder rows | Drag labels |
| Colors | Profile selector (top right): DSView or Classic |

While a pipeline runs, *Viewer* nodes add live rows below the capture channels: digital
traces, decoded-word boxes, and trigger markers.

### Nodes

| Category | Nodes |
|---|---|
| Sources | DSL File Source · Sigrok File Source · DSLogic U3Pro16 (live USB capture) |
| Decoders | SPI Decoder · UART Decoder · Parallel Decoder (1–64-bit bus, SDR/DDR/level sampling) · I2C Decoder (placeholder — editable but not yet runnable) |
| Logic | Packet Framer · Word Field Extractor · Word Matcher (compare/range/set, counting, holdoff/rearm) · Edge Detector · Event Gate · Event Control · SR Flip-Flop · Logic Gate (NOT/AND/OR/XOR/…) · Counter · String Formatter · Buffer |
| Sinks | File Writer · Text File Writer · TGCK Recorder · Viewer |

A typical trigger-and-capture graph: decode SPI commands, match start/stop words, drive an
SR flip-flop that gates a parallel-bus decoder, count captures into generated filenames,
and write each start/stop window to its own file — see
[graphs/spi_controlled_decode.json](graphs/spi_controlled_decode.json).

## Command line & logging

```bash
cargo run --release --bin logic-conduit -- <graph.json>   # open a graph at startup

# Execute exactly the saved graph without opening the UI. This uses the same
# native source factories, output-retention plan, durable caches, and runtime
# as the Run button.
cargo run --release --bin logic-conduit -- run <graph.json>

# Machine-readable timing, throughput, node-progress, and cache report.
cargo run --release --bin logic-conduit -- run <graph.json> --json

# Logging via RUST_LOG (per-module filtering)
RUST_LOG=info cargo run --release --bin logic-conduit
RUST_LOG=info,logic_analyzer_protocol_decoders::spi_decoder=debug cargo run --release --bin logic-conduit
```

Headless execution performs a fresh run just like the UI: it removes that
graph's previous derived-data entries, preserves the raw waveform index, runs
every connected sink, and publishes replacement derived caches. File-writer
nodes write to the destinations saved in the graph. Progress is written to
standard error so JSON standard output can be redirected directly into a
benchmark report.

Reproducible baseline recording and alternating A/B comparisons use the
`performance-regression` developer binary and an external reference capture. See
[`docs/aspects/performance.md`](docs/aspects/performance.md#reproducible-regression-comparisons) for
the workload contract and commands.

If a pipeline appears stuck, the built-in watchdog logs which node is blocked on which
port after ~5 seconds.

## Building & testing

```bash
cargo build --release      # release strongly recommended for capture processing
cargo test                 # workspace tests
```

Run the complete GitHub Actions CI command set locally, or select one workflow job:

```bash
scripts/ci_local.sh
scripts/ci_local.sh clippy
scripts/ci_local.sh check-wasm
```

The jobs run serially and stop at the first failure. On Linux, the script expects the same
`libwayland-dev` system dependency installed by CI. The wasm job requires the setup described
below.

### Testing the browser app on macOS

Install the WebAssembly target and the `wasm-bindgen` CLI once. The CLI version must
match the version pinned by this workspace:

```bash
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.126 --locked
```

Build the browser application and serve its generated files over HTTP:

```bash
scripts/build_wasm_app.sh
python3 -m http.server 8000 --directory target/wasm-app/dist
```

Then open <http://localhost:8000> in Safari, Firefox, or Chrome. Keep the server running
while testing and press `Ctrl-C` in its terminal to stop it. Do not open `index.html`
directly from Finder: browser security rules require the JavaScript modules and WASM
file to be loaded over HTTP.

After changing Rust or web files, run `scripts/build_wasm_app.sh` again and refresh the
browser. For a compile-only check matching CI, run:

```bash
cargo check -p logic-analyzer-app-web --target wasm32-unknown-unknown
```

The repository is a Cargo workspace: `crates/platform_artifacts` (generic byte and artifact
contracts), `crates/platform_runtime` (generic host work and worker contracts),
`crates/signal_runtime` (generic streaming runtime),
`crates/signal_capture`, `crates/signal_derived`, and `crates/signal_capture_session`
(generic signal data-plane contracts),
`crates/logic_analyzer_trigger` and `crates/logic_analyzer_acquisition` (portable
logic-analyzer trigger and acquisition contracts),
the positive-responsibility capture-format, device, decoder, transform, sink, and generator crates,
`crates/logic_analyzer_graph_nodes` (node catalog) and
`crates/logic_analyzer_graph_compiler` (graph compiler),
`crates/widgets/node_graph` (reusable node editor widget),
`crates/widgets/logic_analyzer_viewer` (waveform widget), `crates/logic_analyzer_ui`
(application UI), `crates/app_native` (desktop binary), `crates/app_web`
(browser entry point), and `plugins/example-plugin` (an example
compile-time extension: build with
`--features example-plugin`).

Loadable pipeline examples live in [graphs/](graphs). They include file-backed
SPI processing, direct DSLogic U3Pro16 capture graphs, and the self-contained
[`word_field_extractor_demo.json`](graphs/word_field_extractor_demo.json),
[`packet_framer_demo.json`](graphs/packet_framer_demo.json),
[`event_controls_demo.json`](graphs/event_controls_demo.json), and
[`word_matcher_demo.json`](graphs/word_matcher_demo.json):

The packet-framer demo combines a fixed word count, a Word Matcher Boundary,
and a synthetic capture channel used as an active-high Gate. Its explicit Word
buffer lets the sparse Boundary branch advance independently of packet output
backpressure.

The event-controls demo qualifies rising strobe edges, gates them with a
synthetic signal, and compares automatic holdoff and delay with a branch that
is explicitly rearmed by falling chip-select edges.

The word-matcher demo compares exact, inclusive-range, and set predicates. It
also demonstrates every-Nth-match selection, holdoff, explicit rearming, and
the matching-word output.

```bash
cargo run --release --bin logic-conduit -- graphs/spi_controlled_decode.json
```

The standalone CCD Rust utilities inspect captured image data outside the graph
pipeline:

- `ccd_viewer` opens the verified native-width V500 reconstruction by default:
  54,720 little-endian words per TGCK interval become 18,240 pixels from three
  captured B/R/G groups across four serialized taps. It applies the measured
  color-band row offsets and automatically uses valid N-2/N-1 scanner
  bright-strip/black-level captures for per-lane, per-column calibration. The unfiltered
  reconstruction remains the default. For diagnosis, `--chroma-filter` (or `C`)
  median-filters only the two chroma components in a 3x3 neighborhood while
  preserving each pixel's luminance. Raw and modulo-three diagnostic views
  remain available from the keyboard.
- `ccd_layout_analyzer` compares diagnostic layout views, performs a targeted
  registration across separated image regions, reconstructs the V500's BGR ×
  four-line CCD stream with geometry-gated automatic row-offset fitting, uses
  captured bright-strip/black-level references to select the color-band traversal direction,
  and writes explicit calibration evidence and an accept/reject decision to
  HTML/JSON. TGCK-row modulo views are raster decimations, not CCD-lane proof.

To decode a full native-width image, keep each capture's TGCK boundary CSV next
to its binary file (`capture_NNNN_tgck.csv`). For the recorded V500 sequence,
the analyzer interprets the two immediately preceding captures as the scanner's
final bright-strip and black-level calibration passes. It adopts them only when
they pass dimension, row-count, signal-span, and scene-improvement checks. When
the sibling `capture.csv` and `captures.csv` files are available, the viewer also
requires captures 20, 21, and 22 to share the same recorded frontend offset/gain
settings before applying the profiles. It writes a streaming
16-bit RGB PPM only after the twelve-line geometry and color gates pass:

```bash
cargo run --release -p logic-analyzer-examples --example ccd_layout_analyzer -- \
  --file output/capture_0022.bin \
  --output output/capture_0022-analysis \
  --decoded-image output/capture_0022-decoded-16bit.ppm
```

The PPM contains big-endian 16-bit samples as required by P6, at the complete
18,240-pixel sensor width. It applies the accepted per-lane bright/black profiles
but deliberately performs no display normalization or destructive denoising;
the HTML previews are normalized separately for inspection.

The interactive viewer uses the same lane and calibration model. For the
recorded L scan, run:

```bash
cargo run --release -p logic-analyzer-examples --example ccd_viewer -- \
  --file /Volumes/Extreme/src/linevision/_captures/scan-L-20260824-1835/output/capture_0022.bin
```

The viewer automatically discovers the sibling `decoded/analysis/report.json`
and uses its accepted scan-specific RGB assignment and quarter-row offsets.
Without a compatible report it uses the physical 80/40 nominal offsets.
Explicit color/group arguments override the report; use `--no-analysis-report`
to ignore it entirely. Add `--validate-only` for a headless geometry/reference
check. Pass `--no-calibration` only to inspect uncorrected ADC values.
The chroma filter affects only the interactive 8-bit display. Analyzer output
and captured ADC samples remain unfiltered.
Use `R`, `G`, or `B` to move a color plane by one source row, with Shift to
reverse direction; hold Control for quarter-row steps. Command-line row offsets
also accept quarter rows, for example `--red-row-offset 79.75`.

## Documentation

| Document | Contents |
|---|---|
| [docs/INDEX.md](docs/INDEX.md) | Documentation entry point and crate-owner map |
| [docs/architecture/application_composition.md](docs/architecture/application_composition.md) | UI composition, graph interaction, and live editing |
| [docs/crates/node_graph.md](docs/crates/node_graph.md) | Node editor architecture: model, socket type system, widget |
| `node_graph` Rustdoc | Embedding the editor widget and defining node types (`cargo doc --workspace --no-deps --lib --open`) |
| [docs/crates/logic_analyzer_viewer.md](docs/crates/logic_analyzer_viewer.md) | Waveform viewer: index format, sampling, rendering |
| `logic_analyzer_viewer` Rustdoc | Embedding the viewer widget (`cargo doc --workspace --no-deps --lib --open`) |
| [docs/aspects/live_capture_trigger.md](docs/aspects/live_capture_trigger.md) | Live-capture foundation and staged plan for hardware triggering, capture, and replay |
| `platform_runtime` Rustdoc | Host work and worker-operation contracts (`cargo doc --workspace --no-deps --lib --open`) |
| `signal_runtime` Rustdoc | Streaming engine: nodes, channels, backpressure, live supervision (`cargo doc --workspace --no-deps --lib --open`) |
| [docs/integrations/dslogic_u3pro16_protocol.md](docs/integrations/dslogic_u3pro16_protocol.md) | DSLogic U3Pro16 USB protocol (hardware reference) |
| [docs/references/ccd_afe_registers.md](docs/references/ccd_afe_registers.md) | Hardware register reference |

## Development

This project was developed collaboratively with AI assistance (Codex/Claude/GitHub Copilot).

## License

MIT — see [LICENSE](LICENSE).
