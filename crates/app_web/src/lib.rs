#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn __wasm_call_ctors();
}

#[cfg(target_arch = "wasm32")]
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

#[cfg(any(target_arch = "wasm32", test))]
struct EmbeddedDemo {
    name: &'static str,
    source: &'static str,
    json: &'static str,
}

#[cfg(any(target_arch = "wasm32", test))]
const EMBEDDED_DEMOS: &[EmbeddedDemo] = &[
    EmbeddedDemo {
        name: "Decoder Pipeline",
        source: "crates/app_web/data/wasm_decoder_demo.json",
        json: include_str!("../data/wasm_decoder_demo.json"),
    },
    EmbeddedDemo {
        name: "WASM Decoder",
        source: "graphs/wasm_decoder_demo.json",
        json: include_str!("../../../graphs/wasm_decoder_demo.json"),
    },
    EmbeddedDemo {
        name: "Event Controls",
        source: "graphs/event_controls_demo.json",
        json: include_str!("../../../graphs/event_controls_demo.json"),
    },
    EmbeddedDemo {
        name: "Packet Framer",
        source: "graphs/packet_framer_demo.json",
        json: include_str!("../../../graphs/packet_framer_demo.json"),
    },
    EmbeddedDemo {
        name: "SPI Transactions",
        source: "graphs/spi_transaction_demo.json",
        json: include_str!("../../../graphs/spi_transaction_demo.json"),
    },
    EmbeddedDemo {
        name: "I²C Transactions",
        source: "graphs/i2c_transaction_demo.json",
        json: include_str!("../../../graphs/i2c_transaction_demo.json"),
    },
    EmbeddedDemo {
        name: "Word Field Extractor",
        source: "graphs/word_field_extractor_demo.json",
        json: include_str!("../../../graphs/word_field_extractor_demo.json"),
    },
    EmbeddedDemo {
        name: "Word Matcher",
        source: "graphs/word_matcher_demo.json",
        json: include_str!("../../../graphs/word_matcher_demo.json"),
    },
    EmbeddedDemo {
        name: "Timeline Markers",
        source: "graphs/timeline_markers_demo.json",
        json: include_str!("../../../graphs/timeline_markers_demo.json"),
    },
];

#[cfg(any(target_arch = "wasm32", test))]
fn embedded_demo_graphs() -> Vec<logic_analyzer_ui::DemoGraph> {
    EMBEDDED_DEMOS
        .iter()
        .map(|demo| {
            let graph = serde_json::from_str(demo.json).unwrap_or_else(|error| {
                panic!(
                    "embedded demo '{}' from {} is invalid: {error}",
                    demo.name, demo.source
                )
            });
            logic_analyzer_ui::DemoGraph::new(demo.name, graph)
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        initialize_compile_time_inventories();
        eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    #[wasm_bindgen]
    pub async fn start(&self, canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| {
                    Ok(Box::new(logic_analyzer_ui::App::new_with_demo_graphs(
                        cc,
                        embedded_demo_graphs(),
                    )))
                }),
            )
            .await
    }

    #[wasm_bindgen]
    pub fn destroy(&self) {
        self.runner.destroy();
    }
}

#[cfg(target_arch = "wasm32")]
impl Default for WebHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod demo_graph_tests {
    use std::collections::BTreeSet;

    use super::{EMBEDDED_DEMOS, embedded_demo_graphs};

    #[test]
    fn embedded_demo_contains_its_decoder_panel_layout() {
        let graph: serde_json::Value = serde_json::from_str(EMBEDDED_DEMOS[0].json).unwrap();
        let panels =
            &graph["extensions"]["logic_analyzer_ui.panel_layout"]["decoder_panels"]["panels"];

        assert_eq!(panels.as_object().unwrap().len(), 2);
    }

    #[test]
    fn current_web_demo_remains_the_default_and_available() {
        let demos = embedded_demo_graphs();

        assert_eq!(
            demos.first().map(|demo| demo.name()),
            Some("Decoder Pipeline")
        );
        assert_eq!(demos.len(), EMBEDDED_DEMOS.len());
    }

    #[test]
    fn every_repository_demo_graph_is_embedded() {
        let graph_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../graphs");
        let repository_demos = std::fs::read_dir(graph_directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.ends_with("_demo.json"))
            .collect::<BTreeSet<_>>();
        let embedded_repository_demos = EMBEDDED_DEMOS
            .iter()
            .filter(|demo| demo.source.starts_with("graphs/"))
            .filter_map(|demo| std::path::Path::new(demo.source).file_name())
            .filter_map(|name| name.to_str())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(embedded_repository_demos, repository_demos);
    }

    #[test]
    fn every_embedded_demo_is_valid_and_uses_restorable_nodes() {
        std::hint::black_box(logic_analyzer_graph_nodes::link());
        let registry = logic_analyzer_ui::build_node_registry();
        for demo in EMBEDDED_DEMOS {
            let graph: node_graph::GraphState = serde_json::from_str(demo.json).unwrap();
            for node in graph.nodes.values() {
                assert!(
                    registry.category_of(node.def_name()).is_some(),
                    "demo '{}' uses unregistered node '{}'",
                    demo.name,
                    node.def_name()
                );
                assert!(
                    node.inputs
                        .iter()
                        .chain(&node.outputs)
                        .all(|socket| socket.def_index != usize::MAX),
                    "demo '{}' retains a removed socket on '{}'",
                    demo.name,
                    node.title
                );
                assert!(
                    node.outputs
                        .iter()
                        .all(|socket| !socket.extensions.contains_key("show_in_view")),
                    "demo '{}' retains a legacy viewer selection on '{}'",
                    demo.name,
                    node.title
                );
            }
            let mut widget =
                node_graph::NodeGraphWidget::new(logic_analyzer_ui::build_node_registry());
            widget.set_graph(graph);
            let restored = widget.snapshot_value().unwrap();
            let saved: serde_json::Value = serde_json::from_str(demo.json).unwrap();
            assert_eq!(
                restored, saved,
                "demo '{}' is not saved with the current node schema",
                demo.name
            );
        }
    }
}
