#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        #[cfg(target_os = "macos")]
        mod macos_menu;
        mod native;

        fn main() -> std::process::ExitCode {
            match native::run() {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Error: {error}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
    }
}
