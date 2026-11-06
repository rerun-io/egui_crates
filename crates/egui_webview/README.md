# egui_webview

Web pages as [egui](https://www.egui.rs/) widgets.

This crate is a work in progress.

The app creates a `WebViewInstance` once, keeps it, and calls `WebViewInstance::ui` in every pass in which the page should be shown.
After the pass, it places the page from eframe's output hook:

```rust,ignore
impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.webview.ui(ui, ui.available_size());
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webview.place(ctx, &output.platform_output.native_views);
    }
}
```

Tab and Shift-Tab move the keyboard focus between egui widgets and the page's elements.
`examples/minimal.rs` is a complete app of this shape.

## Backends

* Web: an `<iframe>` below egui's canvas.
* Any native platform, with the `cef` feature: Chromium, through [CEF](https://github.com/chromiumembedded/cef), renders the page off screen into an egui texture, so it works with every egui renderer.
  `WebViewInstance::new` uses it on platforms without a native webview, such as Linux, and when the `EGUI_WEBVIEW_BACKEND` environment variable is `cef`.
  `WebViewInstance::new_cef` uses it on any platform, for example to render with Vulkan on Windows.

The `<iframe>` lies below egui's canvas, and egui clears its rect to transparent so the page shows through.
egui windows and popups above a page paint over it and receive the pointer input there.

## CEF

The [`cef`](https://crates.io/crates/cef) crate downloads CEF (about 1 GB) the first time it is built, and builds a small C++ wrapper library, which requires CMake and Ninja.
Set `CEF_PATH` to share one download between projects.

CEF runs its helper processes from the app's own executable, so call `egui_webview::run_cef_subprocess` at the start of `main`:

```rust,ignore
fn main() {
    if let Some(code) = egui_webview::run_cef_subprocess() {
        std::process::exit(code);
    }
    // …
}
```

* Linux and Windows: the build copies CEF's files next to the binaries in `target/<profile>`.
  A packaged app ships them next to its executable.
* macOS: during development, the app loads CEF from where the build put it.
  A packaged app needs CEF in its app bundle, together with CEF's helper apps.

Pages are rendered on the CPU and uploaded as textures.
JavaScript dialogs, file pickers and drag and drop aren't supported yet.
