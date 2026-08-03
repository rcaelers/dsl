use wasm_bindgen::prelude::*;

use crate::demo_graphs::embedded_demo_graphs;

unsafe extern "C" {
    fn __wasm_call_ctors();
}

fn initialize_compile_time_inventories() {
    static INITIALIZE: std::sync::Once = std::sync::Once::new();
    std::hint::black_box(logic_analyzer_graph_nodes::link());
    #[cfg(feature = "example-plugin")]
    std::hint::black_box(example_plugin::link());
    INITIALIZE.call_once(|| {
        // SAFETY: the linker synthesizes this function for the current WASM
        // module. `Once` guarantees constructors run before the first
        // inventory read and are not repeated by later JS calls.
        unsafe { __wasm_call_ctors() };
    });
}

/// Initializes compile-time inventories in a worker-hosted instance of this WASM module.
///
/// A worker imports the same module without constructing [`WebHandle`], so its
/// linker-provided registrations must be initialized explicitly before any
/// compiler/runtime service is used.
#[wasm_bindgen(js_name = initializeWorkerHost)]
pub fn initialize_worker_host() {
    initialize_compile_time_inventories();
}

#[derive(Clone)]
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
    worker_module_url: String,
    worker_wasm_url: String,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new(worker_module_url: String, worker_wasm_url: String) -> Self {
        initialize_compile_time_inventories();
        eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        Self {
            runner: eframe::WebRunner::new(),
            worker_module_url,
            worker_wasm_url,
        }
    }

    #[wasm_bindgen]
    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        let worker_module_url = self.worker_module_url.clone();
        let worker_wasm_url = self.worker_wasm_url.clone();
        let platform_services = logic_analyzer_platform::standard_services_with_worker_urls(
            &worker_module_url,
            &worker_wasm_url,
        )
        .await;
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let (ui_services, node_catalogs) =
                        platform_services.into_ui_and_node_catalogs();
                    Ok(Box::new(
                        logic_analyzer_ui::App::new_with_demo_graphs_catalogs_and_services(
                            cc,
                            embedded_demo_graphs(),
                            node_catalogs,
                            ui_services,
                        ),
                    ))
                }),
            )
            .await
    }

    #[wasm_bindgen]
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}
