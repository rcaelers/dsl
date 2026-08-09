use logic_analyzer_graph_capabilities::node_support::PortKind;

/// Failure to configure a collected-lane request from registered payload capabilities.
#[derive(Debug, thiserror::Error)]
pub enum PayloadRequestConfigurationError {
    /// No data-subscription contract is registered for the negotiated payload kind.
    #[error("payload {kind:?} has no data-subscription contract")]
    MissingSubscription {
        /// Negotiated payload kind without a subscription contract.
        kind: PortKind,
    },
}

impl PayloadRequestConfigurationError {
    pub(crate) fn missing_subscription(kind: PortKind) -> Self {
        Self::MissingSubscription { kind }
    }
}
