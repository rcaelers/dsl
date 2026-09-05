#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

std::cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        #[cfg(target_os = "macos")]
        mod macos_menu;
        mod native_host;
        mod native_sigrok;
        mod native;
        #[cfg(feature = "developer-tools")]
        mod frame_profile;
        mod sigrok_catalog;
        mod u3pro16_host;

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
