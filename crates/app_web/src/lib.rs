//! Web application composition root for LogicConduit.
//!
//! This crate selects the browser host bootstrap and injects platform services;
//! reusable application policy remains in the workspace library crates.

std::cfg_select! {
    target_arch = "wasm32" => {
        mod demo_graphs;
        mod host_service;
        #[allow(unreachable_pub)]
        mod web;

        pub use web::WebHandle;
    }
    test => {
        mod demo_graphs;
    }
    _ => {}
}
