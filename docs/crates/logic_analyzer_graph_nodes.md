# `logic_analyzer_graph_nodes` Design

## Responsibility

`logic_analyzer_graph_nodes` is the built-in graph-node feature bundle. Each concrete feature owns
its definition, state decoding and migration, builder, presentation metadata, inventory
submission, and feature-local tests.

## Facade and dependencies

The crate root exposes the linker-retention anchor and narrowly scoped constructors for runtime
capability overrides, editor registration overrides, and host-discovered node templates. Internals
depend on graph capabilities, graph registry descriptors, concrete processing, the generic signal
owners, node graph, and viewer registration contracts. Compiler, runtime, and UI read registrations
through the graph registry and never depend on this bundle.

## Ownership boundaries

The crate does not own generic lowering, runtime scheduling, application panel policy, target
selection, or a manual global node list. Stable node and payload identities, plus user-visible
saved-graph migration, remain with their concrete features.

Concrete file-source factories and the Sigrok catalog scanner are bound to individual editor
registries through `GraphNodeEditorOverride`. `NodeDef::on_update` remains host-independent; the
generic node registry runs the bound state update around schema reconciliation. Runtime builders
receive the same application-owned factories through `GraphNodeCapabilityOverride`. No graph-node
factory or scanner is installed in process-global state.

## Built-in socket visual language

Built-in socket definitions use two orthogonal axes through `node_graph`'s type identity table.
Shape communicates the time structure that determines compatibility; color communicates the
payload family.

| Shape | Structure | Meaning |
|---|---|---|
| ■ Square | Static config | One value fixed before a run |
| ● Circle | Level stream | Defined at every instant and transmitted as changes |
| ◆ Diamond | Event stream | Timestamped occurrence, undefined between events |

| Socket type | Runtime stream | Look |
|---|---|---|
| `Signal` | `Sample` or negotiated `SampleBlock` | green ● |
| `Words` | `Word` | orange ◆ |
| `Trigger` | `Trigger` | amber ◆ |
| `Number` | `NumberSample` | blue ● |
| `Text` | `TextSample` | rose ● |
| `Bool` / `Int` / `Float` / `Str` / `File` | static config | square, payload-family hue |
| `Any` | wildcard | grey ● |

A new time structure for an existing payload keeps its hue and changes its shape. A new payload
family receives a new hue. Red is reserved for error feedback and grey for wildcards. The shape
axis provides colorblind robustness because hues that could collide do not share a shape.

## Built-in node inventory

The `sources`, `decoders`, `logic`, and `sinks` directories mirror processing-node families.
Each executable feature directory groups a `node_graph::NodeDef` in `definition.rs`, its separate
graph semantics and runtime materializer in `builder.rs`, and optional presentation metadata. The node body contains the
sockets and controls needed to understand the graph; detailed device and formatter settings belong
in the properties panel. Viewer lane selection and presentation settings belong to the View panel.

Source features cover DSL and Sigrok files and DSLogic U3Pro16 acquisition. Capture-source outputs
opt out of the View-panel lane selector because their capture presentation owns those channels.
Decoder features cover SPI, UART, I2C, parallel words, and catalog-derived Sigrok decoders. Logic
features cover framing, field extraction, matching, edge and event control, state, formatting, and
timeline markers. Output features cover binary, text, CSV, and TGCK recording. The Viewer
registration remains a saved-document compatibility input; current viewing uses UI-owned output
subscriptions and compiler-generated collectors.

The DSLogic feature owns its capture and signal property-panel sections, including the aligned
16-channel enable grid. Invalid channel/rate combinations remain editable for correction, appear
as node errors, and are rejected at materialization. The grid supports anchored Shift-click ranges
and click-drag state painting. Capture-duration presets are constrained by the configuration-derived
maximum: `2^34` samples for streaming or the channel-dependent device depth for buffered capture.

The SPI feature exposes only connectable MOSI/MISO `Words` sockets in the editor. Its compound
Bits/Data lanes are selected through generic presentation metadata and each `Words` socket's eye
summarizes its pair. Word Matcher owns masked comparison, inclusive range and set predicates,
every-Nth selection, holdoff, optional explicit rearm, and matching-word output. Edge Detector owns
edge selection, debounce, and preceding-pulse qualification. Event Gate owns signal-controlled
trigger filtering; Event Control owns delay, holdoff, and optional explicit rearm. Logic Gate's
operation retitles the node and `NOT` limits its variadic group to one socket.

File Writer owns the inline save control when `Filename` is unconnected; a connected text stream
hides that control and takes precedence. Concrete feature documents specify the remaining protocol
settings and behavior.
