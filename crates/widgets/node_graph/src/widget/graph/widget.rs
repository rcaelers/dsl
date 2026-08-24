use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use egui::{Pos2, Sense, Ui};

use input_bindings::InputBindings;

use super::action::HotkeyRegistry;
use super::interaction_state::InteractionState;
use super::menu::MenuController;
use super::panel::PanelState;
use super::response::GraphResponses;
use super::snapshot_error::GraphSnapshotError;
use super::{layout, render};
use crate::api::{
    FileDialogService, PanelAction, PanelDataProvider, PanelTabDef, SocketIndicatorPresentation,
    UnavailableFileDialogService,
};
use crate::model::{FrameId, GraphState, Node, NodeBadge, NodeId, SocketId};
use crate::runtime::{NodeInstance, NodeRuntime, NodeTemplate, NodeTypeRegistry};
use crate::support::{ViewState, graph_position};

// ── Main widget ───────────────────────────────────────────────────────────────

/// Stateful egui widget for editing and presenting a generic node graph.
pub struct NodeGraphWidget {
    pub(crate) graph: GraphState,
    pub(crate) runtime: HashMap<NodeId, Box<dyn NodeInstance>>,
    pub(crate) view: ViewState,
    pub(crate) interaction_state: InteractionState,
    /// Hover context from the most recent frame. Floating panel tabs and
    /// content deliberately leave this empty so hosts do not advertise or
    /// route canvas mouse actions over ordinary widgets.
    pub(crate) hovered_input_context: Option<&'static str>,
    pub(crate) registry: NodeTypeRegistry,
    pub(crate) minimap_visible: bool,
    pub(crate) top_node: Option<NodeId>,
    pub(crate) menu: MenuController,
    /// Pending copy/paste confirmation ("Copied 3 node(s)"), taken and
    /// cleared by the host app's `take_io_status` — the host's own toast
    /// system (Phase 4.2) owns display and timing, not the widget.
    pub(crate) io_status: Option<String>,
    pub(crate) hotkeys: HotkeyRegistry,
    pub(crate) input_bindings: Arc<InputBindings>,
    pub(crate) clipboard_cache: Option<String>,
    pub(crate) undo_stack: Vec<GraphState>,
    pub(crate) redo_stack: Vec<GraphState>,
    pub(crate) frame_rename: Option<FrameRenameState>,
    pub(crate) node_rename: Option<NodeRenameState>,
    /// Most recently clicked/added node; the properties panel shows it.
    pub(crate) active_node: Option<NodeId>,
    pub(crate) panel: PanelState,
    /// Badges set from outside the graph (compiler errors, runtime status);
    /// they take precedence over def-driven badges.
    pub(crate) external_badges: HashMap<NodeId, NodeBadge>,
    /// Short live-status texts (e.g. items-produced counters) drawn small
    /// in the node header.
    pub(crate) node_statuses: HashMap<NodeId, String>,
    /// Nodes whose host-owned derived data can be cleared from the context
    /// menu. The widget only queues a request; the host performs the I/O.
    pub(crate) derived_cache_nodes: HashSet<NodeId>,
    pub(crate) clear_derived_cache_request: Option<NodeId>,
    /// Host-provided, application-neutral node context actions.
    pub(crate) node_context_actions: HashMap<NodeId, Vec<NodeContextAction>>,
    pub(crate) node_context_action_request: Option<(NodeId, String)>,
    pub(crate) contributed_panel_state_changed: bool,
    /// Transient socket decorations grouped by host-owned namespace so one
    /// feature can replace or clear its indicators without touching another.
    pub(crate) socket_indicators: SocketIndicatorRegistry,
    pub(crate) panel_tabs: Vec<PanelTabDef>,
    /// Host-controlled edit gate. View navigation, selection, inspection,
    /// and copy remain available while graph mutations are disabled.
    pub(crate) editing_enabled: bool,
    pub(crate) file_dialog_service: Box<dyn FileDialogService>,
}

/// A context-menu action contributed by the host application. Both the ID
/// and its meaning are opaque to the node graph widget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeContextAction {
    /// Host-owned stable action identifier.
    pub id: String,
    /// User-facing action label.
    pub label: String,
    /// Optional host-provided icon identifier.
    pub icon: Option<String>,
    /// Whether the action is currently shown as checked.
    pub checked: bool,
}

pub(crate) type SocketIndicatorRegistry =
    BTreeMap<String, HashMap<SocketId, BTreeMap<String, Arc<dyn SocketIndicatorPresentation>>>>;

struct EmptyPanelDataProvider;

impl PanelDataProvider for EmptyPanelDataProvider {
    fn panel_data(
        &self,
        _node: NodeId,
        _panel_id: &str,
    ) -> Option<&(dyn std::any::Any + Send + Sync)> {
        None
    }
}

impl NodeContextAction {
    /// Creates an unchecked host context-menu action without an icon.
    ///
    /// # Parameters
    /// - `id`: Host-owned stable action identifier.
    /// - `label`: User-facing action label.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            checked: false,
        }
    }

    /// Sets an optional host-provided icon identifier for the action.
    ///
    /// # Parameters
    /// - `icon`: Opaque icon identifier interpreted by the host renderer.
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets whether the action is rendered as checked.
    ///
    /// # Parameters
    /// - `checked`: Current checked state to display.
    pub fn with_checkmark(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
}

pub(crate) struct FrameRenameState {
    pub(crate) frame_id: FrameId,
    pub(crate) text: String,
    pub(crate) screen_pos: Pos2,
}

pub(crate) struct NodeRenameState {
    pub(crate) node_id: NodeId,
    pub(crate) text: String,
    pub(crate) screen_pos: Pos2,
}

/// Persistable UI state that isn't part of the graph document itself —
/// N-panel width/tab and minimap visibility (Phase 5.2). The host app reads
/// this via [`NodeGraphWidget::ui_prefs`] to save it and restores it via
/// [`NodeGraphWidget::set_ui_prefs`] on the next launch.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphUiPrefs {
    /// Width of the graph widget's docked panel.
    pub panel_width: f32,
    /// Identifier of the selected docked panel tab.
    pub panel_tab: Option<String>,
    /// Whether the graph minimap is visible.
    pub minimap_visible: bool,
}

fn graph_pointer(
    pointer: Option<Pos2>,
    panel_rect: Option<egui::Rect>,
    tab_bar_rect: egui::Rect,
) -> Option<Pos2> {
    pointer.filter(|pointer| {
        !tab_bar_rect.contains(*pointer) && !panel_rect.is_some_and(|rect| rect.contains(*pointer))
    })
}

impl NodeGraphWidget {
    /// Replaces every node template contributed under one host namespace.
    ///
    /// # Parameters
    /// - `namespace`: Stable contributor namespace whose templates are replaced.
    /// - `templates`: Complete replacement template set for that namespace.
    pub fn replace_node_templates(&mut self, namespace: &str, templates: Vec<NodeTemplate>) {
        self.registry.replace_templates(namespace, templates);
    }

    /// Creates an empty editable graph backed by a node-type registry.
    ///
    /// # Parameters
    /// - `registry`: Node definitions and factories available for new and restored nodes.
    pub fn new(registry: NodeTypeRegistry) -> Self {
        let input_bindings = Arc::new(
            InputBindings::from_json(r#"{"bindings":[]}"#)
                .expect("empty input binding configuration is valid"),
        );
        Self {
            graph: GraphState::default(),
            runtime: HashMap::new(),
            view: ViewState::default(),
            interaction_state: InteractionState::default(),
            hovered_input_context: None,
            registry,
            minimap_visible: true,
            top_node: None,
            menu: MenuController::new(),
            io_status: None,
            hotkeys: HotkeyRegistry::graph_defaults(),
            input_bindings,
            clipboard_cache: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            frame_rename: None,
            node_rename: None,
            active_node: None,
            panel: PanelState::default(),
            external_badges: HashMap::new(),
            node_statuses: HashMap::new(),
            derived_cache_nodes: HashSet::new(),
            clear_derived_cache_request: None,
            node_context_actions: HashMap::new(),
            node_context_action_request: None,
            contributed_panel_state_changed: false,
            socket_indicators: BTreeMap::new(),
            panel_tabs: vec![PanelTabDef::new("node", "Node")],
            editing_enabled: true,
            file_dialog_service: Box::new(UnavailableFileDialogService),
        }
    }

    /// Sets file dialog service.
    ///
    /// # Parameters
    /// - `service`: Host adapter used by file-backed node controls.
    pub fn set_file_dialog_service(&mut self, service: Box<dyn FileDialogService>) {
        self.file_dialog_service = service;
    }

    /// Installs the host application's bindings. Context and action names are
    /// opaque to the binding manager and interpreted only by this widget.
    ///
    /// # Parameters
    /// - `input_bindings`: Validated host binding set used for widget interactions.
    pub fn set_input_bindings(&mut self, input_bindings: Arc<InputBindings>) {
        self.input_bindings = input_bindings;
    }

    /// Returns the current persisted graph document.
    pub fn graph(&self) -> &GraphState {
        &self.graph
    }

    /// Returns mutable access to the persisted graph document.
    ///
    /// Call [`Self::sync_node_states`] before persisting or compiling changes made through this
    /// reference when inline controls may still hold uncommitted state.
    pub fn graph_mut(&mut self) -> &mut GraphState {
        &mut self.graph
    }

    /// Reports whether a node-contributed panel changed node state this frame.
    pub fn take_contributed_panel_state_changed(&mut self) -> bool {
        std::mem::take(&mut self.contributed_panel_state_changed)
    }

    /// Returns whether widget-initiated graph mutations are currently allowed.
    pub fn editing_enabled(&self) -> bool {
        self.editing_enabled
    }

    /// Enables or disables graph mutations initiated through the widget.
    /// Disabling during a modal edit restores its pre-edit snapshot.
    ///
    /// # Parameters
    /// - `enabled`: Whether editing commands, drag mutations, and state edits may proceed.
    pub fn set_editing_enabled(&mut self, enabled: bool) {
        if self.editing_enabled == enabled {
            return;
        }
        self.editing_enabled = enabled;
        if enabled {
            return;
        }

        let restore_snapshot = match self.interaction_state {
            InteractionState::DraggingNode { .. }
            | InteractionState::DraggingFrame { .. }
            | InteractionState::PlacingNodes { .. } => true,
            InteractionState::DraggingWire {
                restore_on_cancel, ..
            } => restore_on_cancel,
            InteractionState::Idle
            | InteractionState::Panning { .. }
            | InteractionState::BoxSelecting { .. }
            | InteractionState::CuttingWire { .. } => false,
        };
        if restore_snapshot {
            self.cancel_undo_snapshot();
        }
        if !matches!(
            self.interaction_state,
            InteractionState::Panning { .. } | InteractionState::BoxSelecting { .. }
        ) {
            self.interaction_state = InteractionState::Idle;
        }
        self.frame_rename = None;
        self.node_rename = None;
    }

    /// Flushes inline control state into the graph before an external
    /// operation snapshots or validates it.
    pub fn sync_node_states(&mut self) {
        self.sync_all_node_state();
    }

    /// Takes the pending copy/paste confirmation message, if any — call
    /// once per frame and feed the result into the host app's toast system
    /// (Phase 4.2). Returns `None` most frames.
    pub fn take_io_status(&mut self) -> Option<String> {
        self.io_status.take()
    }

    /// Replaces the nodes whose context menus may request host-owned derived-cache clearing.
    ///
    /// # Parameters
    /// - `nodes`: Node identities with clearable derived data.
    pub fn set_derived_cache_nodes(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        self.derived_cache_nodes = nodes.into_iter().collect();
    }

    /// Takes clear derived cache request, leaving its default state.
    pub fn take_clear_derived_cache_request(&mut self) -> Option<NodeId> {
        self.clear_derived_cache_request.take()
    }

    /// Replaces host-contributed context-menu actions for each node.
    ///
    /// # Parameters
    /// - `actions`: Actions keyed by target node identity.
    pub fn set_node_context_actions(&mut self, actions: HashMap<NodeId, Vec<NodeContextAction>>) {
        self.node_context_actions = actions;
    }

    /// Takes node context action, leaving its default state.
    pub fn take_node_context_action(&mut self) -> Option<(NodeId, String)> {
        self.node_context_action_request.take()
    }

    /// Inserts or replaces a socket decoration owned by a host namespace.
    ///
    /// # Parameters
    /// - `owner`: Namespace isolating this contributor's decorations.
    /// - `socket`: Socket to decorate.
    /// - `id`: Stable decoration identifier within the owner and socket.
    /// - `presentation`: Visual behavior drawn for the decoration.
    pub fn set_socket_indicator(
        &mut self,
        owner: impl Into<String>,
        socket: SocketId,
        id: impl Into<String>,
        presentation: impl SocketIndicatorPresentation,
    ) {
        self.socket_indicators
            .entry(owner.into())
            .or_default()
            .entry(socket)
            .or_default()
            .insert(id.into(), Arc::new(presentation));
    }

    /// Removes one socket decoration without affecting other owners or decorations.
    ///
    /// # Parameters
    /// - `owner`: Namespace that owns the decoration.
    /// - `socket`: Decorated socket.
    /// - `id`: Decoration identifier within the owner and socket.
    pub fn remove_socket_indicator(&mut self, owner: &str, socket: SocketId, id: &str) {
        let Some(by_socket) = self.socket_indicators.get_mut(owner) else {
            return;
        };
        if let Some(indicators) = by_socket.get_mut(&socket) {
            indicators.remove(id);
            if indicators.is_empty() {
                by_socket.remove(&socket);
            }
        }
        if by_socket.is_empty() {
            self.socket_indicators.remove(owner);
        }
    }

    /// Removes every socket decoration owned by one contributor namespace.
    ///
    /// # Parameters
    /// - `owner`: Namespace whose decorations are removed.
    pub fn clear_socket_indicators(&mut self, owner: &str) {
        self.socket_indicators.remove(owner);
    }

    /// Replaces the host-defined tabs. The built-in `Node` tab is always the
    /// first tab and must not be supplied by the host.
    ///
    /// # Parameters
    /// - `tabs`: Host-contributed tabs. Duplicate identifiers are ignored after the first.
    pub fn set_panel_tabs(&mut self, tabs: Vec<PanelTabDef>) {
        let mut seen = HashSet::from(["node".to_owned()]);
        self.panel_tabs = std::iter::once(PanelTabDef::new("node", "Node"))
            .chain(
                tabs.into_iter()
                    .filter(|tab| seen.insert(tab.id().to_owned())),
            )
            .collect();
        if self
            .panel
            .active_tab
            .as_ref()
            .is_some_and(|active| !self.panel_tabs.iter().any(|tab| tab.id() == active))
        {
            self.panel.active_tab = self.panel_tabs.first().map(|tab| tab.id().to_owned());
        }
    }

    /// Current UI prefs (N-panel width/tab, minimap visibility) — for the
    /// host app to persist across launches (Phase 5.2).
    pub fn ui_prefs(&self) -> GraphUiPrefs {
        GraphUiPrefs {
            panel_width: self.panel.width,
            panel_tab: self.panel.active_tab.clone(),
            minimap_visible: self.minimap_visible,
        }
    }

    /// Restores UI prefs saved via [`Self::ui_prefs`] — call once after
    /// construction, before the first `show`.
    ///
    /// # Parameters
    /// - `prefs`: Persisted docked-panel and minimap preferences to restore.
    pub fn set_ui_prefs(&mut self, prefs: GraphUiPrefs) {
        self.panel.width = prefs.panel_width;
        self.panel.active_tab = prefs.panel_tab.map(|requested| {
            self.panel_tabs
                .iter()
                .find(|tab| tab.id() == requested)
                .or_else(|| self.panel_tabs.first())
                .map_or(requested, |tab| tab.id().to_owned())
        });
        self.minimap_visible = prefs.minimap_visible;
    }

    /// Instantiates and adds a registered node template at graph coordinates.
    ///
    /// # Parameters
    /// - `name`: Registered template name, or the built-in `Reroute` name.
    /// - `pos`: Initial graph-space node position.
    ///
    /// Returns the new node identity, or `None` when the template is unknown.
    pub fn add_node_at(&mut self, name: &str, pos: Pos2) -> Option<NodeId> {
        let id = self.graph.next_id();
        if name == "Reroute" {
            let n = Node::new_reroute(id, graph_position(pos));
            let nid = n.id;
            self.graph.add_node(n);
            return Some(nid);
        }
        if let Some(NodeRuntime { node, instance }) = self.registry.instantiate(name, id, pos) {
            let nid = node.id;
            self.runtime.insert(nid, instance);
            self.graph.add_node(node);
            self.set_active_node(nid);
            Some(nid)
        } else {
            None
        }
    }

    pub(crate) fn set_active_node(&mut self, id: NodeId) {
        self.active_node = Some(id);
    }

    /// Replaces a node's state wholesale and re-runs its def (sockets,
    /// visibility, badge) — the programmatic equivalent of editing its
    /// controls. Returns false when the node or its def is unknown or the
    /// state fails to restore.
    pub fn set_node_state(&mut self, id: NodeId, state: serde_json::Value) -> bool {
        let Some(node) = self.graph.nodes.get_mut(&id) else {
            return false;
        };
        node.state = state;
        let Some(instance) = self.registry.restore_node(node) else {
            return false;
        };
        self.runtime.insert(id, instance);
        self.graph.mark_semantic_change();
        true
    }

    /// Applies one host-initiated state edit as an undoable graph mutation.
    pub fn edit_node_state(&mut self, id: NodeId, state: serde_json::Value) -> bool {
        if !self.editing_enabled {
            return false;
        }
        self.sync_all_node_state();
        if self.graph.nodes.get(&id).map(|node| &node.state) == Some(&state) {
            return true;
        }
        let previous = self.graph.clone();
        if self.set_node_state(id, state) {
            self.undo_stack.push(previous);
            self.redo_stack.clear();
            true
        } else {
            self.graph = previous;
            self.restore_runtime();
            false
        }
    }

    /// Sets (or clears, with `None`) an externally owned badge on a node —
    /// compile errors, runtime status. External badges render instead of the
    /// def's own badge while present.
    pub fn set_node_badge(&mut self, id: NodeId, badge: Option<NodeBadge>) {
        match badge {
            Some(badge) => {
                self.external_badges.insert(id, badge);
            }
            None => {
                self.external_badges.remove(&id);
            }
        }
    }

    /// Sets (or clears) the short live-status text drawn in a node's header
    /// (e.g. "1.2M" items while a pipeline runs).
    ///
    /// # Parameters
    /// - `id`: Input consumed by this operation.
    /// - `status`: Input consumed by this operation.
    pub fn set_node_status(&mut self, id: NodeId, status: Option<String>) {
        match status {
            Some(status) => {
                self.node_statuses.insert(id, status);
            }
            None => {
                self.node_statuses.remove(&id);
            }
        }
    }

    /// Clears every live-status text (e.g. when a new run starts).
    pub fn clear_node_statuses(&mut self) {
        self.node_statuses.clear();
    }

    pub(crate) fn fit_graph_to_viewport(
        &mut self,
        layout: &layout::GraphWidgetLayout,
        viewport: egui::Rect,
        origin: Pos2,
    ) {
        let bounds = layout
            .node_rects
            .values()
            .chain(layout.frame_rects.values())
            .copied()
            .reduce(|bounds, rect| bounds.union(rect));
        if let Some(bounds) = bounds {
            self.view.fit_to_rect(bounds, viewport, origin, 48.0);
        } else {
            self.view = ViewState::default();
        }
    }

    /// Zooms to fit the current selection (Phase 2, Blender's numpad-`.`) —
    /// falls back to fitting the whole graph, matching `Home`, when nothing
    /// is selected.
    pub(crate) fn fit_selection_to_viewport(
        &mut self,
        layout: &layout::GraphWidgetLayout,
        viewport: egui::Rect,
        origin: Pos2,
    ) {
        let node_bounds = self
            .graph
            .nodes
            .values()
            .filter(|node| node.selected)
            .filter_map(|node| layout.node_rects.get(&node.id).copied());
        let frame_bounds = self
            .graph
            .frames
            .iter()
            .filter(|frame| frame.selected)
            .filter_map(|frame| layout.frame_rects.get(&frame.id).copied());
        let bounds = node_bounds.chain(frame_bounds).reduce(|a, b| a.union(b));
        match bounds {
            Some(bounds) => self.view.fit_to_rect(bounds, viewport, origin, 48.0),
            None => self.fit_graph_to_viewport(layout, viewport, origin),
        }
    }

    /// Replaces the whole graph and rebuilds every node's runtime instance
    /// from the registry — the programmatic equivalent of loading a saved
    /// file. State restore runs through the same reconcile path as
    /// file loading (`restore_node`): sockets validated against current defs,
    /// `on_update` re-run, badges recomputed.
    pub fn set_graph(&mut self, graph: GraphState) {
        let previous_revision = self.graph.semantic_revision();
        self.graph = graph;
        self.graph.reconcile_reroute_outputs();
        self.graph.mark_semantic_change_after(previous_revision);
        self.contributed_panel_state_changed = false;
        self.external_badges.clear();
        self.node_statuses.clear();
        self.active_node = None;
        self.restore_runtime();
    }

    /// Resets to a fresh, empty graph — the programmatic equivalent of
    /// File → New (Phase 5.1). Clears undo/redo along with graph content;
    /// UI prefs (panel width, minimap) and the runtime registry are
    /// untouched.
    pub fn new_graph(&mut self) {
        self.set_graph(GraphState::default());
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Captures the current graph, including state still held by inline node
    /// controls. Used by document persistence and dirty-state tracking.
    pub fn snapshot_value(&mut self) -> Result<serde_json::Value, GraphSnapshotError> {
        self.sync_all_node_state();
        serde_json::to_value(&self.graph).map_err(GraphSnapshotError::from)
    }

    pub(crate) fn run_update(&mut self, id: NodeId) {
        let before = self.graph.nodes.get(&id).map(|node| {
            serde_json::to_vec(&(&node.inputs, &node.outputs, node.muted, &node.state))
                .expect("node semantic state is always serializable")
        });
        if let (Some(instance), Some(node)) =
            (self.runtime.get_mut(&id), self.graph.nodes.get_mut(&id))
        {
            instance.update(&mut node.inputs, &mut node.outputs);
            if let Some(title) = instance.bound_title() {
                node.title = title;
            }
            node.state = instance.save_state();
            node.badge = instance.badge();
        }
        let changed = before.is_some_and(|before| {
            self.graph.nodes.get(&id).is_some_and(|node| {
                before
                    != serde_json::to_vec(&(&node.inputs, &node.outputs, node.muted, &node.state))
                        .expect("node semantic state is always serializable")
            })
        });
        if changed {
            self.graph.mark_semantic_change();
        }
    }

    pub(crate) fn sync_all_node_state(&mut self) {
        let mut changed = false;
        for id in self.graph.sorted_node_ids() {
            if let (Some(instance), Some(node)) =
                (self.runtime.get_mut(&id), self.graph.nodes.get_mut(&id))
            {
                instance.set_bound_title(&node.title);
                let state = instance.save_state();
                changed |= node.state != state;
                node.state = state;
            }
        }
        if changed {
            self.graph.mark_semantic_change();
        }
    }

    pub(crate) fn push_undo_snapshot(&mut self) {
        self.sync_all_node_state();
        self.undo_stack.push(self.graph.clone());
        self.redo_stack.clear();
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub(crate) fn restore_runtime(&mut self) {
        self.runtime.clear();
        for node in self.graph.nodes.values_mut() {
            if let Some(instance) = self.registry.restore_node(node) {
                self.runtime.insert(node.id, instance);
            }
        }
    }

    // ── Viewport render ───────────────────────────────────────────────────────

    /// Draws the graph using no host-supplied panel data.
    ///
    /// # Parameters
    /// - `ui`: Parent UI receiving the graph canvas and floating panels.
    pub fn show(&mut self, ui: &mut Ui) -> Vec<PanelAction> {
        self.show_with_panel_data(ui, &EmptyPanelDataProvider)
    }

    /// Draws the graph using host-owned panel models borrowed for this call.
    /// Every contributed-panel action emitted during the draw is returned in
    /// creation order; neither models nor actions are retained by the widget.
    ///
    /// # Parameters
    /// - `ui`: Parent UI receiving the graph canvas and floating panels.
    /// - `panel_data`: Host data provider queried while drawing contributed panels.
    pub fn show_with_panel_data(
        &mut self,
        ui: &mut Ui,
        panel_data: &dyn PanelDataProvider,
    ) -> Vec<PanelAction> {
        let mut panel_actions = Vec::new();
        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let origin = rect.min;

        let pointer = response
            .hover_pos()
            .or_else(|| ui.input(|i| i.pointer.hover_pos()));

        // The right-side tab strip is always present. The optional panel body
        // floats over the graph and only claims input where it is visible.
        let tab_bar_rect = self.panel_tab_bar_rect(rect);
        let panel_rect = self.panel_rect(rect, panel_data);
        let content_rect =
            egui::Rect::from_min_max(rect.min, Pos2::new(tab_bar_rect.left(), rect.max.y));
        let layout = self.build_layout(origin);
        let responses = if self.interaction_state.use_fast_rendering() {
            GraphResponses::canvas_only(response)
        } else {
            self.allocate_responses(ui, response, &layout, content_rect)
        };

        // Register the floating UI after every graph hit target so it owns
        // overlapping clicks and drags in egui's interaction z-order.
        if let Some(panel_rect) = panel_rect {
            self.update_panel_interaction(ui, panel_rect);
        }
        self.update_panel_tab_bar_interaction(ui, tab_bar_rect);

        let graph_pointer = graph_pointer(pointer, panel_rect, tab_bar_rect);
        self.hovered_input_context = graph_pointer.map(|_| "node_graph");
        let hovered_socket = graph_pointer.and_then(|_| self.hovered_socket(&responses));
        self.handle_input(ui, &responses, graph_pointer, origin, &layout, content_rect);

        let layout = self.build_layout(origin);
        self.draw_graph(
            ui,
            &painter,
            render::GraphRenderContext {
                rect: content_rect,
                origin,
                pointer,
                layout: &layout,
                hovered_socket,
            },
        );
        self.show_socket_tooltip(&responses, hovered_socket);
        if let Some(panel_rect) = panel_rect {
            self.show_active_panel(ui, panel_rect, panel_data, &mut panel_actions);
        }
        self.show_panel_tab_bar(ui, tab_bar_rect);
        self.show_frame_rename(ui.ctx());
        self.show_node_rename(ui.ctx());
        panel_actions
    }

    /// One-line hint of available actions for the current interaction
    /// state, for a status bar (Phase 4.1). Static strings only — cheap
    /// enough to call every frame.
    pub fn status_hint(&self) -> &'static str {
        match &self.interaction_state {
            InteractionState::DraggingWire { .. } => {
                "Release on a socket to connect · release on canvas to search for a node"
            }
            InteractionState::PlacingNodes { .. } => "Click to place · Esc to cancel",
            InteractionState::CuttingWire { .. } => "Release to cut the crossed wires",
            InteractionState::DraggingNode { .. } => "Drop inside a frame to join it",
            InteractionState::DraggingFrame { .. }
            | InteractionState::BoxSelecting { .. }
            | InteractionState::Panning { .. } => "",
            InteractionState::Idle => {
                let any_selected = self.graph.nodes.values().any(|node| node.selected)
                    || self.graph.frames.iter().any(|frame| frame.selected);
                if any_selected {
                    "Shift+D Duplicate · F2 Rename · H Collapse · X Delete · . Zoom to Selection"
                } else {
                    "Shift+A Add · A Select All · RMB Menu · MMB Pan"
                }
            }
        }
    }

    /// Most-specific input-binding context for an active graph interaction.
    ///
    /// The strings are opaque to the generic binding manager. Returning only
    /// active interactions lets the host keep ordinary hover context handling
    /// separate while ensuring a drag remains active after leaving the graph
    /// rectangle.
    pub fn active_input_context(&self) -> Option<&'static str> {
        match self.interaction_state {
            InteractionState::DraggingNode { .. } => Some("node_graph.drag_node"),
            InteractionState::DraggingWire { .. } => Some("node_graph.drag_wire"),
            _ => None,
        }
    }

    /// Input-binding context under the pointer in the most recently rendered
    /// frame. Panel tabs and panel content are widget-owned and return `None`.
    pub fn hovered_input_context(&self) -> Option<&'static str> {
        self.hovered_input_context
    }

    /// Current zoom level as a whole-number percentage, for a status bar.
    pub fn zoom_percent(&self) -> i32 {
        (self.view.zoom * 100.0).round() as i32
    }

    /// `"n nodes"` or `"m/n selected"`, for a status bar.
    pub fn selection_summary(&self) -> String {
        let total = self.graph.nodes.len();
        let selected = self
            .graph
            .nodes
            .values()
            .filter(|node| node.selected)
            .count();
        if selected > 0 {
            format!("{selected}/{total} selected")
        } else {
            format!("{total} node{}", if total == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use egui::{Painter, Pos2, Rect, Vec2};

    use super::{GraphUiPrefs, NodeGraphWidget, graph_pointer};
    use crate::api::{
        FileDialogRequest, FileDialogService, PanelTabDef, SocketIndicatorPresentation,
    };
    use crate::model::{NodeId, SocketDirection, SocketId};
    use crate::runtime::NodeTypeRegistry;
    use crate::support::graph_position;
    use crate::widget::graph::action::GraphAction;
    use crate::widget::graph::interaction_state::InteractionState;

    struct TestIndicator;

    impl SocketIndicatorPresentation for TestIndicator {
        fn size(&self, _zoom: f32) -> Vec2 {
            Vec2::splat(8.0)
        }

        fn draw(&self, _painter: &Painter, _rect: Rect, _zoom: f32) {}
    }

    struct TestFileDialog {
        availability_checks: Arc<AtomicUsize>,
    }

    impl FileDialogService for TestFileDialog {
        fn available(&self, _save: bool) -> bool {
            self.availability_checks.fetch_add(1, Ordering::Relaxed);
            true
        }

        fn pick(&mut self, _request: FileDialogRequest<'_>) -> Option<String> {
            None
        }
    }

    #[test]
    fn file_dialog_setter_installs_the_host_service() {
        let availability_checks = Arc::new(AtomicUsize::new(0));
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        widget.set_file_dialog_service(Box::new(TestFileDialog {
            availability_checks: Arc::clone(&availability_checks),
        }));

        assert!(widget.file_dialog_service.available(false));
        assert_eq!(availability_checks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn node_panel_is_open_by_default_and_restored_preferences_win() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        assert_eq!(widget.ui_prefs().panel_tab.as_deref(), Some("node"));

        widget.set_ui_prefs(GraphUiPrefs {
            panel_width: 280.0,
            panel_tab: None,
            minimap_visible: true,
        });
        assert_eq!(widget.ui_prefs().panel_tab, None);
    }

    #[test]
    fn tabs_are_widget_configuration_and_stale_preferences_fall_back() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        widget.set_panel_tabs(vec![PanelTabDef::new("diagnostics", "Diagnostics")]);
        assert_eq!(widget.panel_tabs.len(), 2);
        assert_eq!(widget.panel_tabs[0].id(), "node");
        assert_eq!(widget.panel_tabs[0].label(), "Node");

        widget.set_panel_tabs(vec![
            PanelTabDef::new("node", "Host override"),
            PanelTabDef::new("diagnostics", "Diagnostics"),
            PanelTabDef::new("diagnostics", "Duplicate"),
        ]);
        assert_eq!(widget.panel_tabs.len(), 2);
        assert_eq!(widget.panel_tabs[0].label(), "Node");

        widget.set_ui_prefs(GraphUiPrefs {
            panel_width: 300.0,
            panel_tab: Some("removed-tab".to_owned()),
            minimap_visible: true,
        });
        assert_eq!(widget.ui_prefs().panel_tab.as_deref(), Some("node"));
    }

    #[test]
    fn socket_indicator_owners_can_replace_and_remove_their_own_decorations() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let socket = SocketId {
            node: NodeId(7),
            index: 2,
            direction: SocketDirection::Output,
        };
        widget.set_socket_indicator("feature-a", socket, "active", TestIndicator);
        widget.set_socket_indicator("feature-b", socket, "warning", TestIndicator);
        assert_eq!(widget.socket_indicators.len(), 2);

        widget.remove_socket_indicator("feature-a", socket, "active");
        assert!(!widget.socket_indicators.contains_key("feature-a"));
        assert!(widget.socket_indicators.contains_key("feature-b"));

        widget.clear_socket_indicators("feature-b");
        assert!(widget.socket_indicators.is_empty());
    }

    #[test]
    fn floating_panel_blocks_graph_pointer_only_inside_its_bounds() {
        let panel = Rect::from_min_max(Pos2::new(600.0, 0.0), Pos2::new(900.0, 400.0));
        let tabs = Rect::from_min_max(Pos2::new(900.0, 0.0), Pos2::new(924.0, 800.0));

        assert_eq!(
            graph_pointer(Some(Pos2::new(700.0, 200.0)), Some(panel), tabs),
            None
        );
        assert_eq!(
            graph_pointer(Some(Pos2::new(910.0, 200.0)), Some(panel), tabs),
            None
        );
        assert_eq!(
            graph_pointer(Some(Pos2::new(700.0, 500.0)), Some(panel), tabs),
            Some(Pos2::new(700.0, 500.0))
        );
        assert_eq!(
            graph_pointer(Some(Pos2::new(300.0, 200.0)), Some(panel), tabs),
            Some(Pos2::new(300.0, 200.0))
        );
    }

    #[test]
    fn floating_panel_widgets_do_not_report_the_canvas_input_context() {
        fn context_at(pointer: Pos2) -> Option<&'static str> {
            let context = egui::Context::default();
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1_000.0, 600.0));
            context.begin_pass(egui::RawInput {
                screen_rect: Some(rect),
                events: vec![egui::Event::PointerMoved(pointer)],
                ..Default::default()
            });
            let mut ui = egui::Ui::new(
                context.clone(),
                egui::Id::new("graph-hover-context-test"),
                egui::UiBuilder::new().max_rect(rect),
            );
            let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
            widget.show(&mut ui);
            let hovered = widget.hovered_input_context();
            let mut output = context.end_pass();
            output.textures_delta.clear();
            hovered
        }

        assert_eq!(context_at(Pos2::new(200.0, 200.0)), Some("node_graph"));
        assert_eq!(context_at(Pos2::new(750.0, 20.0)), None);
        assert_eq!(context_at(Pos2::new(990.0, 20.0)), None);
    }

    #[test]
    fn node_drag_reports_a_specific_input_context() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        assert_eq!(widget.active_input_context(), None);

        widget.interaction_state = InteractionState::DraggingNode {
            node_id: NodeId(1),
            offset: Vec2::ZERO,
            constraint: None,
        };
        assert_eq!(widget.active_input_context(), Some("node_graph.drag_node"));
    }

    #[test]
    fn read_only_mode_blocks_mutations_but_keeps_selection_actions() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget.add_node_at("Reroute", Pos2::ZERO).unwrap();
        widget.set_editing_enabled(false);

        widget.execute_action(
            GraphAction::Delete { target: Some(node) },
            &egui::Context::default(),
            None,
        );
        assert!(widget.graph().nodes.contains_key(&node));

        widget.execute_action(GraphAction::SelectAll, &egui::Context::default(), None);
        assert!(widget.graph().nodes[&node].selected);
    }

    #[test]
    fn entering_read_only_mode_reverts_an_active_node_drag() {
        let mut widget = NodeGraphWidget::new(NodeTypeRegistry::new());
        let node = widget
            .add_node_at("Reroute", Pos2::new(10.0, 20.0))
            .unwrap();
        widget.push_undo_snapshot();
        widget.graph.nodes.get_mut(&node).unwrap().pos = graph_position(Pos2::new(80.0, 90.0));
        widget.interaction_state = InteractionState::DraggingNode {
            node_id: node,
            offset: Vec2::ZERO,
            constraint: None,
        };

        widget.set_editing_enabled(false);

        assert!(!widget.editing_enabled());
        assert_eq!(
            widget.graph().nodes[&node].pos,
            graph_position(Pos2::new(10.0, 20.0))
        );
        assert!(matches!(widget.interaction_state, InteractionState::Idle));
    }
}
