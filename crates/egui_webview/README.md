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
        self.webview.place(ctx, &output.platform_output.embedded_views);
    }
}
```

Tab and Shift-Tab move the keyboard focus between egui widgets and the page's elements.
`examples/minimal_instance.rs` is a complete app of this shape.

## Initialization

Call `egui_webview::init` first in `main`:

```rust,ignore
fn main() {
    egui_webview::init();
    // …
}
```

## Showing a page without keeping it

A `WebViewInstance` isn't `Send`, so not every piece of code can keep one.
`WebView` shows a page with one call and keeps nothing:

```rust,ignore
let response = egui_webview::WebView::new(egui::Id::unique("docs"))
    .url("https://docs.rs/egui")
    .show(ui);
```

The pages then live in a `WebViews` that the app owns and updates from eframe's output hook:

```rust,ignore
impl eframe::App for MyApp {
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webviews.update(ctx, output);
    }
}
```

`examples/minimal.rs` is a complete app of this shape.

* The id is global. The same id shows the same page from anywhere in the app.
* A page lives only while it is shown. Call `WebView::keep_alive` in every frame to keep a hidden page.
* `WebView::reload`, `WebView::load` and the other commands take the id, so they work from anywhere too.

## Running the examples

Natively:

```sh
cargo run -p egui_webview --example minimal
cargo run -p egui_webview --example minimal_instance
```

The examples run on the web with [Trunk](https://github.com/trunk-rs/trunk):

```sh
cd crates/egui_webview
trunk serve --example minimal
trunk serve --example minimal_instance
```

`crates/egui_webview_demo` is a bigger demo, with a browser bar, pages in egui windows and a log of the pages' events.

## Backends

* Web: an `<iframe>` below egui's canvas.
* macOS, with the `wry` feature (on by default): a `WKWebView`, created through [wry](https://github.com/tauri-apps/wry), below egui's surface.
  It needs eframe's wgpu renderer.
* Any native platform, with the `cef` feature: Chromium, through [CEF](https://github.com/chromiumembedded/cef), renders the page off screen into an egui texture, so it works with every egui renderer.
  `WebViewInstance::new` uses it on platforms without another backend, such as Linux and Windows, and when the `EGUI_WEBVIEW_BACKEND` environment variable is `cef`.
  With both `wry` and `cef` enabled, `wry` wins on macOS unless that variable is set.
  `WebViewInstance::new_cef` uses it on any platform, for example to render with Vulkan on Windows.
  `WebViewInstance::new_cef_with_options` also takes `CefOptions`: the page's frame rate, and the background color of pages that set none, which is white by default.

The embedded views (web, macOS) lie below egui's surface, and egui clears their rects to transparent so the pages show through.
egui windows and popups above a page paint over it and receive the pointer input there.

Every backend gives the page egui's theme as its `prefers-color-scheme`, and updates it when the theme changes.
So pages that follow the OS theme, such as ones built with Pico CSS or Tailwind's `dark:`, follow the app's theme instead.

## CEF

The [`cef`](https://crates.io/crates/cef) crate downloads CEF (about 1 GB) the first time it is built, and builds a small C++ wrapper library, which requires CMake and Ninja.
Set `CEF_PATH` to share one download between projects.

CEF runs its helper processes from the app's own executable, which is why `egui_webview::init` must run first in `main`.
With the `cef` feature it calls `egui_webview::cef_init`: in a helper process that runs CEF and exits, and otherwise it returns right away.

`egui_webview::run_cef_subprocess` does the same as `cef_init` without exiting, and returns the helper's exit code.

* Linux and Windows: the build copies CEF's files next to the binaries in `target/<profile>`.
  A packaged app ships them next to its executable.
* macOS: during development, the app loads CEF from where the build put it.
  A packaged app needs CEF in its app bundle, together with CEF's helper apps.

Pages are rendered on the CPU and uploaded as textures.
JavaScript dialogs, file pickers and drag and drop aren't supported yet.
