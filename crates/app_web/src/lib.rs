std::cfg_select! {
    target_arch = "wasm32" => {
        mod demo_graphs;
        mod web;

        pub use web::WebHandle;
    }
    test => {
        mod demo_graphs;
    }
    _ => {}
}
