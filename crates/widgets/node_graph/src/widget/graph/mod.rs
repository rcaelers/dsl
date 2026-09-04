mod action;
mod connection_paint;
mod input_dispatch;
mod interaction;
mod interaction_state;
mod layout;
mod menu;
mod minimap;
mod panel;
mod render;
mod response;
mod routing;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "layout routing adapter is activated by the next implementation step"
    )
)]
mod routing_input;
mod selection;
mod snapshot_error;
mod widget;
mod wire;

pub use snapshot_error::GraphSnapshotError;
pub(crate) use widget::SocketIndicatorRegistry;
pub use widget::{GraphUiPrefs, NodeContextAction, NodeGraphWidget};
