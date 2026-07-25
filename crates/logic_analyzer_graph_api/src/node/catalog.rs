use std::path::PathBuf;

use node_graph::api::NodeTemplate;

#[derive(Clone, Debug, Default)]
pub struct NodeCatalogStatus {
    pub scanning: bool,
    pub discovered: usize,
    pub diagnostics: Vec<String>,
}

/// A host-configurable source of preconfigured graph-node templates.
///
/// Implementations own discovery and persistence. The UI only presents the
/// generic directory collection and installs completed template snapshots.
pub trait DirectoryNodeCatalog: Send {
    fn namespace(&self) -> &str;
    fn title(&self) -> &str;
    fn directories(&self) -> Vec<PathBuf>;
    fn set_directories(&mut self, directories: Vec<PathBuf>);
    fn rescan(&mut self);
    fn status(&mut self) -> NodeCatalogStatus;
    fn take_templates(&mut self) -> Option<Vec<NodeTemplate>>;
}
