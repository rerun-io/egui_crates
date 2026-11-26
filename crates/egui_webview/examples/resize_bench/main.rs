//! Measures how far the frames of a CEF page lag behind the widget while it is resized.
//!
//! The page's height moves up and down at a constant speed for a few seconds. In every pass, the
//! frame that egui shows has the height that the widget had some time ago. That time is the lag.
//!
//! Prints the lag, how many frames CEF painted, and how many of them were dropped because they
//! had a stale size. Compare the numbers before and after a change to how the CEF backend
//! resizes, pumps or paints. They only compare on the same machine and display.
//!
//! ```sh
//! cargo run --release -p egui_webview --features cef --example resize_bench
//! ```
//!
//! Options:
//!
//! - `--fps <n>`: `CefOptions::frame_rate`.
//! - `--still`: don't resize, and only repaint when the page asks for it. Shows how many passes
//!   a page costs while nothing happens.
//! - `--animated`: the page runs a CSS animation. With `--still`, this shows the frame rate of
//!   a page that drives the repaints itself.

cfg_select! {
    target_arch = "wasm32" => {
        fn main() {}
    }
    _ => {
        mod native;

        fn main() -> eframe::Result {
            native::run()
        }
    }
}
