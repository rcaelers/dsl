use std::sync::Arc;

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
    let logic_analyzer_platform::WorkerGraphHostServices {
        output_storage,
        dsl_file_source_factory,
        sigrok_file_source_factory,
    } = logic_analyzer_platform::worker_graph_host_services();
    let capability_overrides = vec![
        logic_analyzer_graph_nodes::binary_file_writer_capability_override(
            logic_analyzer_processing::nodes::sinks::binary_file_writer::writer_factory(
                Arc::clone(&output_storage),
            ),
        ),
        logic_analyzer_graph_nodes::csv_word_writer_capability_override(
            logic_analyzer_processing::nodes::sinks::csv_word_writer::writer_factory(Arc::clone(
                &output_storage,
            )),
        ),
        logic_analyzer_graph_nodes::text_file_writer_capability_override(
            logic_analyzer_processing::nodes::sinks::text_file_writer::writer_factory(
                output_storage,
            ),
        ),
        logic_analyzer_graph_nodes::dsl_file_source_capability_override(dsl_file_source_factory),
        logic_analyzer_graph_nodes::sigrok_file_source_capability_override(
            sigrok_file_source_factory,
        ),
    ];
    logic_analyzer_platform::initialize_graph_worker_runtime(
        logic_analyzer_graph_orchestration::GraphWorkerRuntime::with_repository(
            capability_overrides,
            logic_analyzer_platform::worker_artifact_repository(),
        ),
    );
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
    /// Creates the web application shell and installs browser host services.
    ///
    /// # Parameters
    /// - `worker_module_url`: Input consumed by this operation.
    /// - `worker_wasm_url`: Input consumed by this operation.
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
    /// Starts the web application in the supplied browser canvas.
    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        let worker_module_url = self.worker_module_url.clone();
        let worker_wasm_url = self.worker_wasm_url.clone();
        let platform_services = logic_analyzer_platform::standard_services_with_worker_urls(
            logic_analyzer_ui::APPLICATION_ID,
            &worker_module_url,
            &worker_wasm_url,
        )
        .await;
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let (ui_services, node_catalogs) = application_services(platform_services);
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
    /// Tears down the running web application and releases browser resources.
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

fn application_services(
    platform_services: logic_analyzer_platform::PlatformServices,
) -> (
    logic_analyzer_ui::AppServices,
    Vec<Box<dyn logic_analyzer_ui::NodeCatalogService>>,
) {
    let logic_analyzer_platform::PlatformServices {
        node_file_dialog,
        capture_worker_client,
        app_manager_factory,
        dsl_file_source_factory,
        sigrok_file_source_factory,
        graph_worker_client,
        artifact_repository,
        work_executor,
        worker_operation_executor,
        ..
    } = platform_services;

    logic_analyzer_graph_nodes::install_file_source_factories(
        Arc::clone(&dsl_file_source_factory),
        Arc::clone(&sigrok_file_source_factory),
    );
    let capability_overrides = vec![
        logic_analyzer_graph_nodes::dsl_file_source_capability_override(dsl_file_source_factory),
        logic_analyzer_graph_nodes::sigrok_file_source_capability_override(
            sigrok_file_source_factory,
        ),
    ];
    let source_preparation_executor: Box<
        dyn logic_analyzer_graph_runtime::SourcePreparationExecutor,
    > = if let Some(client) = capture_worker_client {
        Box::new(
            logic_analyzer_graph_runtime::CaptureWorkerSourcePreparationExecutor::new(
                client,
                Box::new(logic_analyzer_graph_runtime::InlineSourcePreparationExecutor),
            ),
        )
    } else {
        Box::new(logic_analyzer_graph_runtime::InlineSourcePreparationExecutor)
    };
    let ui_services = logic_analyzer_ui::AppServices::with_host_configuration(
        Box::new(crate::host_service::BrowserHostService::new()),
        logic_analyzer_ui::default_input_bindings(),
        logic_analyzer_ui::ApplicationSettings::default(),
        Vec::new(),
    )
    .with_capture_export_service(logic_analyzer_ui::unavailable_capture_export_service())
    .with_node_file_dialog(
        node_file_dialog.expect("the browser supplies its asynchronous node-file dialog"),
    )
    .with_graph_execution_and_capability_overrides(
        source_preparation_executor,
        app_manager_factory,
        Arc::clone(&work_executor),
        capability_overrides,
    )
    .with_graph_worker_client(graph_worker_client)
    .with_worker_operation_executor(worker_operation_executor)
    .with_artifact_repository(artifact_repository);
    (ui_services, Vec::new())
}

#[cfg(test)]
mod web_tests {
    use super::WebHandle;

    #[wasm_bindgen_test::wasm_bindgen_test(unsupported = test)]
    fn inventory_restores_embedded_demo_socket_names() {
        let _handle = WebHandle::new(
            "test-worker-module.js".to_owned(),
            "test-worker-module.wasm".to_owned(),
        );
        let graph = serde_json::from_str(include_str!("../data/wasm_decoder_demo.json"))
            .expect("the default web demo is valid");
        let mut widget = node_graph::NodeGraphWidget::new(logic_analyzer_ui::build_node_registry());
        widget.set_graph(graph);

        for node in widget.graph().nodes.values() {
            for socket in node.inputs.iter().chain(&node.outputs) {
                assert!(
                    !socket.name.is_empty(),
                    "{} retains an unnamed socket after web restoration",
                    node.title
                );
            }
        }
    }
}
