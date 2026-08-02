# Node Graph Widget — API

How to embed and extend the `node-graph` crate ([crates/widgets/node_graph](../crates/widgets/node_graph)).
For the internal architecture see [NODE_GRAPH_DESIGN.md](NODE_GRAPH_DESIGN.md).

Public surface (crate root re-exports):

```rust
pub use api::{
    AnySocket, BoolSocket, BoolValue, EnumValue, FileSocket, FileValue, FloatSocket,
    FloatValue, InlineControl, InputDef, IntSocket, IntValue, NodeDef, OutputDef,
    PanelSection, PropDef, SocketDef, SocketTypeIdentity, SocketWithControlDef,
    StrSocket, StringValue,
};
pub use model::{
    BadgeSeverity, Connection, Frame, FrameId, GraphState, Node, NodeBadge, NodeId,
    NodeKind, Socket, SocketDirection, SocketId, SocketShape, VariadicInfo,
};
pub use runtime::{NodeTypeRegistry, SocketTypeStyle};
pub use widget::NodeGraphWidget;
```

---

## Getting started

```rust
use node_graph::{NodeGraphWidget, NodeTypeRegistry};

let mut registry = NodeTypeRegistry::new();
registry.register::<MyNode>();          // one call per NodeDef type
let mut widget = NodeGraphWidget::new(registry);

// per frame, inside your egui layout:
widget.show(ui);                        // fills the available rect
```

The widget is self-contained: it owns the graph document, undo/redo, clipboard,
interaction, context menus, minimap, and the properties panel.

## Defining a socket type

A socket type gives wires and sockets an identity (name, color, shape). Implement
`SocketDef` once per type; the registry auto-collects identities from every node def that
mentions the type (first registration wins), so the type looks identical graph-wide.

```rust
use node_graph::{SocketDef, SocketShape};

pub struct Words;
impl SocketDef for Words {
    type Value = u64;                                // carried value (host semantics)
    fn type_name() -> &'static str { "Words" }
    fn color() -> Color32 { Color32::from_rgb(215, 140, 60) }
    fn shape() -> SocketShape { SocketShape::Diamond } // default: Circle
}
```

`SocketWithControlDef` additionally binds a control type so an unconnected input can be
edited inline:

```rust
pub trait SocketWithControlDef: SocketDef {
    type Control: InlineControl;
}
```

Builtins: `BoolSocket`, `IntSocket`, `FloatSocket`, `StrSocket`, `FileSocket` (all with
controls) and `AnySocket` (the wildcard: compatible with everything; used by reroutes).
Their value types — `BoolValue`, `IntValue` (with optional range), `FloatValue` (range +
drag speed), `StringValue`, `FileValue` (open/save intent, title and filters, asynchronous host
completion, and file drop),
`EnumValue` (variant list; **persists by variant name**, so saved files survive reorders) —
are serde types you embed directly in node state.

Custom controls implement `InlineControl`:

```rust
pub trait InlineControl: Send + Sync + fmt::Debug {
    /// Draw into `rect` at the graph `zoom`; return true when the value changed.
    fn draw_widget(&mut self, ui: &mut Ui, label: &str, rect: Rect, zoom: f32,
                   clip_rect: Rect) -> bool;
}
```

## Defining a node type

A node type is a `NodeDef`: a serde `State` plus static socket/prop descriptions.

```rust
use node_graph::{
    InputDef, IntValue, NodeDef, NodePanelDef, OutputDef, PanelSection, PropDef,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterState { pub start: IntValue, pub step: IntValue }

pub struct Counter;
impl NodeDef for Counter {
    type State = CounterState;

    fn name() -> &'static str { "Counter" }       // unique; menu + serialization key
    fn category() -> &'static str { "Logic" }     // add-menu grouping
    fn color() -> Color32 { COLOR_LOGIC }         // header color

    fn inputs() -> Vec<InputDef<Self::State>> {
        vec![InputDef::new::<Trigger>("Trigger")]
    }
    fn outputs() -> Vec<OutputDef<Self::State>> {
        vec![OutputDef::new::<Number>("Count")]
    }
    fn state() -> Self::State {
        CounterState { start: IntValue::plain(0), step: IntValue::plain(1) }
    }
    fn panel() -> Vec<PanelSection<Self::State>> {
        vec![PanelSection::new("Options", vec![
            PropDef::control("start", "Start", |s| &mut s.start),
            PropDef::control("step",  "Step",  |s| &mut s.step),
        ])]
    }
    fn panels() -> Vec<NodePanelDef<Self::State>> {
        vec![] // optional node-owned presentations assigned to widget tabs
    }
}
```

### `InputDef` / `OutputDef` builders

| Constructor / method | Effect |
|---|---|
| `InputDef::new::<T>(label)` | Plain input of socket type `T` |
| `InputDef::control::<T>(label, accessor)` | Input whose unconnected state renders `T::Control` inline, bound to a state field |
| `.accepts::<U>()` | This input also accepts `U` — the node handles it itself (e.g. a constant on a stream input). Connecting a `U` sets the socket's `resolved_type`, restyling socket + wire to `U`'s identity |
| `.idle_style(color, shape)` | Override the unconnected look (resolved look always comes from the connected type) |
| `.variadic(max)` | Growing group: members "{label} 1…N" plus a trailing placeholder; connecting the placeholder adds a member. No inline controls |
| `OutputDef::new::<T>(label)` / `::control::<T>(…)` | Same for outputs (outputs never resolve; they keep their concrete type) |
| `OutputDef::editor_visible(false)` | Keeps an output available to host processing without drawing its unused socket row; an existing connection still reveals it |

### Properties, tabs, and panels

- `props()` → `Vec<PropDef>` render in the **node body** (zoom-scaled, keep them few).
- `panel()` → `Vec<PanelSection>` render in the right-docked **properties panel**
  (screen-space, full-size widgets) when the node is active. `PropDef::panel_height(h)`
  requests a taller row (e.g. a channel grid).
- Tabs are widget configuration (`PanelTabDef`) and remain visible for every node and when no node
  is active. The widget always supplies the first `Node` tab; the host configures only additional
  tabs such as `View`.
- `panels()` → `Vec<NodePanelDef>` contributes zero or more panels to those tabs. Each definition
  supplies a stable panel ID, a tab ID, optional height/scroll metadata, and a
  `NodePanelPresentation`. The presentation draws the complete content, including its title and
  empty state; `node_graph` only allocates its rectangle and optional scroll area.
- The built-in Node panel is the only panel whose contents are controlled by `node_graph`. It
  contains node identity plus the properties returned by `panel()`.
- `NodeDef::title()` can opt into binding the displayed node title to one state-owned
  `StringValue`; the built-in Name editor and inline properties then edit one synchronized value.
- A panel presentation can inspect opaque, typed host data through `PanelContext::data` and emit
  opaque typed actions through `PanelContext::emit`. The data and action types belong to the
  concrete feature; `node_graph` does not define their fields or meaning.
- Host panel models are draw-scoped. The host passes a `PanelDataProvider` to
  `NodeGraphWidget::show_with_panel_data`; the widget borrows matching `(NodeId, panel ID)` values
  for height calculation and drawing, and never retains them. The host owns model construction,
  replacement, cleanup, and any persistence outside node state.
- `show_with_panel_data` returns every `PanelAction` emitted during that draw. The host handles the
  actions immediately; the widget does not queue actions between frames. `show` is the convenience
  form for hosts with no contributed panel data and uses an empty provider.
- State edits from node-body properties, the built-in Node panel, and node-contributed panels all
  trigger the same update path.

### Socket indicators

Transient decorations next to inputs and outputs use the generic
`SocketIndicatorPresentation` contract. A presentation reports its screen-space size and draws
inside the rectangle allocated by the graph widget. The widget supports multiple indicators per
socket and groups them by host-owned namespace, with `set_socket_indicator`,
`remove_socket_indicator`, and `clear_socket_indicators` APIs. Indicator meaning, iconography,
color, and persistence remain outside `node_graph`.

### Hooks

```rust
fn on_update(state: &mut State, inputs: &mut [Socket], outputs: &mut [Socket]) {}
fn badge(state: &State) -> Option<NodeBadge> { None }
```

`on_update` runs after every state edit, connect, disconnect, and restore — use it for
dynamic socket visibility (`socket.visible`), restyling (`color`/`shape`), and clamping
interdependent props. `badge` is recomputed after each update; return
`NodeBadge::info/warning/error(text)` to draw a status line under the node.

## `NodeGraphWidget` reference

| Method | Purpose |
|---|---|
| `new(registry)` | Create with a populated `NodeTypeRegistry` |
| `show(ui)` | Render + handle one frame in `ui`'s available rect |
| `graph()` / `graph_mut()` | Access the `GraphState` document (e.g. for compilation) |
| `add_node_at(name, pos) -> Option<NodeId>` | Programmatic add (`"Reroute"` adds a reroute) |
| `set_node_state(id, json) -> bool` | Replace a node's state and re-run its def (sockets, visibility, badge) |
| `set_graph(graph)` | Replace the document; rebuilds all runtime instances via restore reconciliation |
| `snapshot_value()` | Synchronize inline controls and return the graph document for host persistence |
| `set_node_badge(id, Option<NodeBadge>)` | Externally owned badge (compile errors, runtime status); takes precedence over the def badge |
| `set_node_status(id, Option<String>)` / `clear_node_statuses()` | Short live text in node headers (e.g. item counters) |

`NodeTypeRegistry`: `new()`, `register::<T: NodeDef>()` (chainable), `category_of(name)`,
`socket_type_style(name) -> Option<SocketTypeStyle>`.

## Model types

`GraphState { nodes, connections, frames, … }` is plain serde data — safe to inspect and
(carefully) mutate. Hosts can preserve namespaced document state through
`extension::<T>(key)`, `set_extension(key, value)`, and `remove_extension(key)`; generic graph
code serializes these values without interpreting them. Useful pieces:

- `Node`: `id`, `title`, `pos`, `state: serde_json::Value`, `inputs`/`outputs:
  Vec<Socket>`, `kind` (`Regular`/`Reroute`), `collapsed`, `selected`, `badge`.
- `Socket`: `effective_type()` (resolved-or-native type name), `accepts(type_name)`
  (the compatibility rule), `is_variadic_member()`, `is_variadic_placeholder()`.
- `Connection { from: SocketId, to: SocketId }`; `SocketId { node, direction, index }` is
  positional per node side.
- `GraphState::is_input_connected(socket_id)`, `sorted_node_ids()`.

## Built-in interaction (for reference)

Right-click opens the context menu (add search, cut/copy/paste, duplicate, delete,
dissolve, frame operations, show/hide sockets, collapse, undo/redo). Keyboard: `A` add
search at pointer · `⌘X/C/V` cut/copy/paste (system clipboard, JSON payload) · `⇧D`
duplicate · `Delete`/`Backspace`/`X` delete · `⌘Z`/`⇧⌘Z` undo/redo · `⌘J` join in frame ·
`⌘H` hide unconnected sockets · `M` minimap · `N` properties panel · `Esc` cancel.
Hotkeys are suppressed while any widget holds keyboard focus.
