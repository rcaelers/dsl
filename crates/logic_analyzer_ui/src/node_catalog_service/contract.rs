use node_graph::api::NodeTemplate;

/// Portable UI snapshot of one host-managed graph-node catalog.
#[derive(Clone, Debug, Default)]
pub struct NodeCatalogSnapshot {
    /// Stable namespace used to replace this catalog's templates.
    pub namespace: String,
    /// User-facing catalog title.
    pub title: String,
    /// Host-formatted directory labels in configured order.
    pub directories: Vec<String>,
    /// Whether discovery is currently in progress.
    pub scanning: bool,
    /// Number of templates found by the latest completed scan.
    pub discovered: usize,
    /// Non-fatal diagnostics suitable for presentation.
    pub diagnostics: Vec<String>,
}

/// Host implementation of one portable dynamic node-catalog service.
///
/// The host owns directory selection, persistence, and scanning. UI consumers issue catalog
/// actions by stable identity and never receive filesystem paths or scanner implementations.
pub trait NodeCatalogService: Send {
    /// Returns the latest presentation snapshot and polls completed host work.
    fn snapshot(&mut self) -> NodeCatalogSnapshot;
    /// Opens the host directory selector and adds the selected directory, when any.
    fn add_directory(&mut self);
    /// Removes one configured directory by its snapshot index.
    ///
    /// # Parameters
    /// - `index`: Ordered directory index from the latest snapshot.
    fn remove_directory(&mut self, index: usize);
    /// Starts a new host discovery scan.
    fn rescan(&mut self);
    /// Takes the latest completed template snapshot, if one is available.
    fn take_templates(&mut self) -> Option<Vec<NodeTemplate>>;
}
