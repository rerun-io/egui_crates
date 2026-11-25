//! Types into a page that CEF renders, by injecting events into egui's input, and checks the
//! values of the page's form fields after each step.
//!
//! A plugin adds the events to egui's input, as an integration would. The OS sees none of them,
//! and the window opens inactive, so the test can run while you use the computer.
//!
//! Prints `PASS` and exits with code 0 if the page reports the expected values after every step.
//!
//! ```sh
//! cargo run -p egui_webview --features cef --example cef_input_test
//! ```

cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        mod native;
        #[path = "../synthetic/mod.rs"]
        mod synthetic;

        fn main() -> eframe::Result {
            native::run()
        }
    }
}
