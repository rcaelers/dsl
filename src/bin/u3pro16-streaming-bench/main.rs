//! Generated-transport U3Pro16 streaming benchmark.

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        fn main() {
            logic_analyzer_device_dslogic::run_streaming_benchmark();
        }
    }
}
