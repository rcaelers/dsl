use std::fmt;
use std::marker::PhantomData;

use egui::{Color32, Rect, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::builtins::StringValue;
use super::control::{FileDialogService, InlineControl, InlineControlContext};
use super::panel::NodePanelDef;
use super::socket::{SocketDef, SocketWithControlDef};
use crate::model::{NodeBadge, Socket, SocketShape};

/// Identity of a socket type as it should appear graph-wide: used to re-skin
/// sockets that resolved to this type, regardless of any per-def idle styling.
#[derive(Debug, Clone)]
pub struct SocketTypeIdentity {
    /// Stable type name used for graph-wide compatibility and resolved styling.
    pub name: &'static str,
    /// Type-family color used once a socket resolves to this type.
    pub color: Color32,
    /// Type-family shape used once a socket resolves to this type.
    pub shape: SocketShape,
}

impl SocketTypeIdentity {
    fn of<T: SocketDef>() -> Self {
        Self {
            name: T::type_name(),
            color: T::color(),
            shape: T::shape(),
        }
    }
}

/// Declarative schema for one input socket on a node definition.
pub struct InputDef<S> {
    pub(crate) stable_id: String,
    pub(crate) label: String,
    pub(crate) type_name: &'static str,
    /// Idle look shown while unconnected; defaults to the native type's
    /// identity, overridable per def via [`InputDef::idle_style`].
    pub(crate) color: Color32,
    pub(crate) shape: SocketShape,
    /// Native type identity (never restyled) — feeds the type identity table.
    pub(crate) identity: SocketTypeIdentity,
    /// Extra types this input accepts; the node handles them itself.
    pub(crate) accepted: Vec<SocketTypeIdentity>,
    /// `Some(max)` turns this def into a growing group: it starts as a single
    /// placeholder socket; each connection converts the placeholder into a
    /// member and spawns a new one, up to `max` members.
    pub(crate) variadic_max: Option<usize>,
    pub(crate) control: Option<Box<dyn ControlBinding<S>>>,
}

impl<S: 'static> InputDef<S> {
    /// Creates an input definition with the socket type's default presentation.
    ///
    /// # Parameters
    /// - `label`: User-facing label and default stable socket identity.
    pub fn new<T: SocketDef>(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            stable_id: label.clone(),
            label,
            type_name: T::type_name(),
            color: T::color(),
            shape: T::shape(),
            identity: SocketTypeIdentity::of::<T>(),
            accepted: Vec::new(),
            variadic_max: None,
            control: None,
        }
    }

    /// Creates an input with an inline default-value control.
    ///
    /// # Parameters
    /// - `label`: User-facing label and default stable socket identity.
    /// - `accessor`: Selects the control value from mutable node state.
    pub fn control<T: SocketWithControlDef>(
        label: impl Into<String>,
        accessor: for<'a> fn(&'a mut S) -> &'a mut T::Control,
    ) -> Self {
        let label = label.into();
        Self {
            stable_id: label.clone(),
            label: label.clone(),
            type_name: T::type_name(),
            color: T::color(),
            shape: T::shape(),
            identity: SocketTypeIdentity::of::<T>(),
            accepted: Vec::new(),
            variadic_max: None,
            control: Some(Box::new(ControlBindingRenderer { label, accessor })),
        }
    }

    /// Sets the persisted schema identity independently of the display label.
    pub fn stable_id(mut self, stable_id: impl Into<String>) -> Self {
        self.stable_id = stable_id.into();
        self
    }

    /// Declares that this input also accepts `T` — the node's processing is
    /// able to handle a `T` on this input (e.g. a constant on a stream input).
    pub fn accepts<T: SocketDef>(mut self) -> Self {
        self.accepted.push(SocketTypeIdentity::of::<T>());
        self
    }

    /// Overrides the look shown while the socket is unconnected. The resolved
    /// look always comes from the connected type's identity.
    pub fn idle_style(mut self, color: Color32, shape: SocketShape) -> Self {
        self.color = color;
        self.shape = shape;
        self
    }

    /// Turns this input into a growing group of up to `max` sockets.
    /// Connecting to the trailing placeholder adds a member ("{label} 1",
    /// "{label} 2", …) and a new placeholder; disconnecting a member removes
    /// it. Variadic inputs cannot carry inline controls.
    pub fn variadic(mut self, max: usize) -> Self {
        self.variadic_max = Some(max.max(1));
        self
    }
}

/// Declarative schema for one output socket on a node definition.
pub struct OutputDef<S> {
    pub(crate) stable_id: String,
    pub(crate) label: String,
    pub(crate) type_name: &'static str,
    pub(crate) color: Color32,
    pub(crate) shape: SocketShape,
    pub(crate) identity: SocketTypeIdentity,
    pub(crate) control: Option<Box<dyn ControlBinding<S>>>,
    pub(crate) editor_visible: bool,
}

impl<S: 'static> OutputDef<S> {
    /// Creates an output definition with the socket type's default presentation.
    pub fn new<T: SocketDef>(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            stable_id: label.clone(),
            label,
            type_name: T::type_name(),
            color: T::color(),
            shape: T::shape(),
            identity: SocketTypeIdentity::of::<T>(),
            control: None,
            editor_visible: true,
        }
    }

    /// Creates an output definition with an inline control bound to node state.
    ///
    /// # Parameters
    /// - `label`: User-facing label and default stable socket identity.
    /// - `accessor`: Selects the control value from mutable node state.
    pub fn control<T: SocketWithControlDef>(
        label: impl Into<String>,
        accessor: for<'a> fn(&'a mut S) -> &'a mut T::Control,
    ) -> Self {
        let label = label.into();
        Self {
            stable_id: label.clone(),
            label: label.clone(),
            type_name: T::type_name(),
            color: T::color(),
            shape: T::shape(),
            identity: SocketTypeIdentity::of::<T>(),
            control: Some(Box::new(ControlBindingRenderer { label, accessor })),
            editor_visible: true,
        }
    }

    /// Sets the persisted schema identity independently of the display label.
    pub fn stable_id(mut self, stable_id: impl Into<String>) -> Self {
        self.stable_id = stable_id.into();
        self
    }

    /// Controls whether this output has a socket row in the node editor.
    /// Connected outputs remain visible so existing wires stay editable.
    pub fn editor_visible(mut self, visible: bool) -> Self {
        self.editor_visible = visible;
        self
    }
}

type ControlAccessor<S, T> = for<'a> fn(&'a mut S) -> &'a mut T;

pub(crate) trait ControlBinding<S> {
    fn draw(
        &self,
        state: &mut S,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool;
}

struct ControlBindingRenderer<S, T> {
    label: String,
    accessor: ControlAccessor<S, T>,
}

impl<S, T: InlineControl> ControlBinding<S> for ControlBindingRenderer<S, T> {
    fn draw(
        &self,
        state: &mut S,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        (self.accessor)(state).draw_widget(
            ui,
            &self.label,
            rect,
            zoom,
            clip_rect,
            &mut InlineControlContext::new(file_dialog),
        )
    }
}

struct InstanceControlBindingRenderer<S, T, F> {
    label: String,
    accessor: F,
    marker: PhantomData<fn(&mut S) -> &mut T>,
}

impl<S, T, F> ControlBinding<S> for InstanceControlBindingRenderer<S, T, F>
where
    T: InlineControl,
    F: for<'a> Fn(&'a mut S) -> &'a mut T,
{
    fn draw(
        &self,
        state: &mut S,
        ui: &mut Ui,
        rect: Rect,
        zoom: f32,
        clip_rect: Rect,
        file_dialog: &mut dyn FileDialogService,
    ) -> bool {
        (self.accessor)(state).draw_widget(
            ui,
            &self.label,
            rect,
            zoom,
            clip_rect,
            &mut InlineControlContext::new(file_dialog),
        )
    }
}

/// Declarative binding between a node-state field and an inline control.
pub struct PropDef<S> {
    pub(crate) id: String,
    /// Row height when rendered in a side panel; `None` uses the
    /// panel's default row height. Controls that need more vertical room
    /// (e.g. a channel grid) set this.
    pub(crate) panel_height: Option<f32>,
    pub(crate) binding: Box<dyn ControlBinding<S>>,
}

impl<S: 'static> PropDef<S> {
    /// Binds an inline control to a node-state property.
    ///
    /// # Parameters
    /// - `id`: Stable property identity within the node schema.
    /// - `label`: User-facing control label.
    /// - `accessor`: Selects the control value from mutable node state.
    pub fn control<T: InlineControl + 'static>(
        id: impl Into<String>,
        label: impl Into<String>,
        accessor: for<'a> fn(&'a mut S) -> &'a mut T,
    ) -> Self {
        let label = label.into();
        Self {
            id: id.into(),
            panel_height: None,
            binding: Box::new(ControlBindingRenderer { label, accessor }),
        }
    }

    /// Binds a control selected from instance state. Unlike [`Self::control`],
    /// the accessor may capture stable schema data such as an option index.
    pub fn instance_control<T, F>(
        id: impl Into<String>,
        label: impl Into<String>,
        accessor: F,
    ) -> Self
    where
        T: InlineControl + 'static,
        F: for<'a> Fn(&'a mut S) -> &'a mut T + Send + Sync + 'static,
    {
        Self {
            id: id.into(),
            panel_height: None,
            binding: Box::new(InstanceControlBindingRenderer {
                label: label.into(),
                accessor,
                marker: PhantomData,
            }),
        }
    }

    /// Requests a taller row in a side panel.
    pub fn panel_height(mut self, height: f32) -> Self {
        self.panel_height = Some(height);
        self
    }
}

/// A titled, collapsible group of controls in a side panel.
pub struct PanelSection<S> {
    /// User-facing title of the collapsible group.
    pub title: String,
    /// Controls rendered within the group.
    pub props: Vec<PropDef<S>>,
}

impl<S> PanelSection<S> {
    /// Creates a titled group of controls for the node side panel.
    pub fn new(title: impl Into<String>, props: Vec<PropDef<S>>) -> Self {
        Self {
            title: title.into(),
            props,
        }
    }
}

/// Complete socket and control schema for one saved node instance.
/// Complete socket and control schema materialized for one saved node state.
pub struct NodeInstanceSchema<S> {
    /// Input socket definitions.
    pub inputs: Vec<InputDef<S>>,
    /// Output socket definitions.
    pub outputs: Vec<OutputDef<S>>,
    /// Inline property controls.
    pub props: Vec<PropDef<S>>,
    /// Traditional side-panel control sections.
    pub panel: Vec<PanelSection<S>>,
    /// Node-contributed widget-level panels.
    pub panels: Vec<NodePanelDef<S>>,
}

impl<S> NodeInstanceSchema<S> {
    /// Creates a schema with inputs and outputs and no controls or panels.
    pub fn new(inputs: Vec<InputDef<S>>, outputs: Vec<OutputDef<S>>) -> Self {
        Self {
            inputs,
            outputs,
            props: Vec::new(),
            panel: Vec::new(),
            panels: Vec::new(),
        }
    }

    /// Adds inline property controls to the schema.
    ///
    /// # Parameters
    /// - `props`: Inline controls to bind to this node state.
    pub fn props(mut self, props: Vec<PropDef<S>>) -> Self {
        self.props = props;
        self
    }

    /// Adds traditional side-panel sections to the schema.
    pub fn panel(mut self, panel: Vec<PanelSection<S>>) -> Self {
        self.panel = panel;
        self
    }

    /// Adds node-contributed widget-level panels to the schema.
    pub fn panels(mut self, panels: Vec<NodePanelDef<S>>) -> Self {
        self.panels = panels;
        self
    }
}

/// Generic add-menu metadata for one node category path.
///
/// Category paths use `::` to describe nested menus. `root_order` orders only
/// the first path segment; categories with the same value retain registry
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddMenuCategory {
    path: String,
    root_order: i32,
}

impl AddMenuCategory {
    /// Creates a category at the default root-menu position.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            root_order: 0,
        }
    }

    /// Creates a category with an explicit root-menu ordering value.
    ///
    /// Lower values appear first. Equal values preserve registry order.
    pub fn ordered(path: impl Into<String>, root_order: i32) -> Self {
        Self {
            path: path.into(),
            root_order,
        }
    }

    /// Returns the `::`-separated user-facing category path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the ordering value for the first category path segment.
    pub fn root_order(&self) -> i32 {
        self.root_order
    }
}

/// Compile-time definition of a concrete node type and its persisted state.
pub trait NodeDef: 'static {
    /// Serializable node-owned configuration state.
    type State: fmt::Debug + Clone + Serialize + DeserializeOwned + 'static;

    /// Returns the stable registered node-type name.
    fn name() -> &'static str
    where
        Self: Sized;
    /// Returns the user-facing category used by node creation menus.
    fn category() -> &'static str
    where
        Self: Sized;
    /// Returns generic add-menu metadata for this node's category.
    fn add_menu_category() -> AddMenuCategory
    where
        Self: Sized,
    {
        AddMenuCategory::new(Self::category())
    }
    /// Returns whether the node is offered in the add-node menu.
    fn add_menu_visible() -> bool
    where
        Self: Sized,
    {
        true
    }
    /// Returns the default node-header color.
    fn color() -> Color32
    where
        Self: Sized,
    {
        Color32::from_rgb(80, 80, 80)
    }
    /// Returns the static input socket schema.
    fn inputs() -> Vec<InputDef<Self::State>>
    where
        Self: Sized;
    /// Returns the static output socket schema.
    fn outputs() -> Vec<OutputDef<Self::State>>
    where
        Self: Sized;
    /// Creates default persisted state for a new node instance.
    fn state() -> Self::State
    where
        Self: Sized;
    /// Opts this node into using one state string as its displayed title.
    /// Inline edits and the generic Node-panel name editor then operate on
    /// the same value.
    fn title(state: &mut Self::State) -> Option<&mut StringValue>
    where
        Self: Sized,
    {
        let _ = state;
        None
    }
    /// Returns the deterministic schema for one saved state. Static node
    /// definitions inherit the traditional methods; plugin-owned dynamic
    /// definitions override this method and keep their schema snapshot in
    /// state.
    fn instance_schema(state: &Self::State) -> NodeInstanceSchema<Self::State>
    where
        Self: Sized,
    {
        let _ = state;
        NodeInstanceSchema::new(Self::inputs(), Self::outputs())
            .props(Self::props())
            .panel(Self::panel())
            .panels(Self::panels())
    }
    /// Returns inline property controls for static node definitions.
    fn props() -> Vec<PropDef<Self::State>>
    where
        Self: Sized,
    {
        vec![]
    }
    /// Properties shown in the right-docked properties panel when this node
    /// is active. Edits run through the same state/`on_update` path as
    /// inline controls.
    fn panel() -> Vec<PanelSection<Self::State>>
    where
        Self: Sized,
    {
        vec![]
    }
    /// Panels contributed by this node. Each presentation owns all content;
    /// the graph widget only places it in the requested widget-level tab.
    fn panels() -> Vec<NodePanelDef<Self::State>>
    where
        Self: Sized,
    {
        vec![]
    }
    /// Recomputes definition-owned state-dependent socket schema and validation.
    ///
    /// # Parameters
    /// - `state`: Mutable node state after a user edit or restore.
    /// - `inputs`: Materialized input sockets that may be updated for the state.
    /// - `outputs`: Materialized output sockets that may be updated for the state.
    fn on_update(_state: &mut Self::State, _inputs: &mut [Socket], _outputs: &mut [Socket])
    where
        Self: Sized,
    {
    }
    /// Rewrites node-owned socket identities from older saved schemas before
    /// generic reconciliation matches them against the current definitions.
    /// Implementations also record any user-visible compatibility warning in
    /// their state.
    fn migrate_saved_sockets(
        _state: &mut Self::State,
        _inputs: &mut Vec<Socket>,
        _outputs: &mut Vec<Socket>,
    ) where
        Self: Sized,
    {
    }
    /// Status message shown under the node, recomputed after every state
    /// update (validation notes, clamped settings, …).
    fn badge(_state: &Self::State) -> Option<NodeBadge>
    where
        Self: Sized,
    {
        None
    }
}
