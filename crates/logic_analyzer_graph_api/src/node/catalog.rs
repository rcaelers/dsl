use std::path::PathBuf;

use node_graph::api::NodeTemplate;

#[derive(Clone, Debug, Default)]
/// Latest discovery state for one host-configured node-template catalog.
pub struct NodeCatalogStatus {
    /// Whether a discovery scan is currently in progress.
    pub scanning: bool,
    /// Number of templates found by the most recent completed scan.
    pub discovered: usize,
    /// Non-fatal discovery diagnostics suitable for presentation to the host.
    pub diagnostics: Vec<String>,
}

/// A host-configurable source of preconfigured graph-node templates.
///
/// Implementations own discovery and persistence. The UI only presents the
/// generic directory collection and installs completed template snapshots.
pub trait DirectoryNodeCatalog: Send {
    /// Returns the stable namespace used to scope this catalog's templates.
    fn namespace(&self) -> &str;
    /// Returns the user-facing catalog title.
    fn title(&self) -> &str;
    /// Returns the configured discovery directories.
    fn directories(&self) -> Vec<PathBuf>;
    /// Replaces the configured discovery directories.
    ///
    /// # Parameters
    /// - `directories`: New ordered set of locations to scan for templates.
    fn set_directories(&mut self, directories: Vec<PathBuf>);
    /// Starts a new discovery scan.
    fn rescan(&mut self);
    /// Returns the latest scan status.
    fn status(&mut self) -> NodeCatalogStatus;
    /// Takes the latest completed template snapshot, if one is available.
    fn take_templates(&mut self) -> Option<Vec<NodeTemplate>>;
}
