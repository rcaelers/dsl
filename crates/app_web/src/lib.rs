//! Web application composition root for LogicConduit.
//!
//! This crate selects browser mechanisms and application fallbacks, assembles
//! concrete graph capabilities, and injects the resulting UI services.

std::cfg_select! {
    target_arch = "wasm32" => {
        mod demo_graphs;
        mod host_service;
        mod node_file_dialog;
        #[allow(unreachable_pub)]
        mod web;

        pub use web::WebHandle;
    }
    test => {
        mod demo_graphs;
        mod node_file_dialog;
    }
    _ => {}
}
