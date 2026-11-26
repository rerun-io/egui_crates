# egui_webview

Web pages as [egui](https://www.egui.rs/) widgets.

> [!WARNING]
> This crate is a work in progress.

# Backends

| Backend | Feature | Windows          | macOS             | Linux | Web |
|---------|---------|------------------|-------------------|-------|-----|
| Wry     |  `wry`  | ✅<br>(WebView2) | ✅<br>(WKWebView) | ❌    | ❌  |
| CEF     |  `cef`  | ✅               | ✅                | ✅    | ❌  |
| iframe  |   N/A   | ❌               | ❌                | ❌    | ✅  |

Using the `default_backend` feature (enabled by default) will choose the first supported backend, in order of the table above.

Every backend gives the page egui's theme as its `prefers-color-scheme`, and updates it when the theme changes.
So pages that follow the OS theme, such as ones built with Pico CSS or Tailwind's `dark:`, follow the app's theme instead.

# Getting Started



## Initialization

Call `egui_webview::init` first in `main`:

```rust,ignore
fn main() {
    egui_webview::init();
    // …
}
```

## Showing a WebViewInstance

Create and store a `WebViewInstance` in your app, call `WebViewInstance::ui` to show it, and call `WebViewInstance::place` from eframe's output hook.

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
> [!TIP]
> See `examples/minimal_instance.rs` for a full example using `WebViewInstance`.

## Showing a WebView

A `WebViewInstance` isn't `Send`, which can be annoying to persist.
`WebView` lets you show a page with a single call:

```rust,ignore
let response = egui_webview::WebView::new(egui::Id::unique("docs"))
    .url("https://docs.rs/egui")
    .show(ui);
```

The pages are stored for you in `WebViews`, which the app owns & updates from eframe's output hook:

```rust,ignore
impl eframe::App for MyApp {
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webviews.update(ctx, output);
    }
}
```

> [!TIP]
> See `examples/minimal.rs` for a full example using `WebView`/`WebViews`.

* The id is global. The same id shows the same page from anywhere in the app.
* A page lives only while it is shown. Call `WebView::keep_alive` in every frame to keep a hidden page.
* `WebView::reload`, `WebView::load` and the other commands take the id, so they work from anywhere too.

# Running the examples

Natively:

```sh
cargo run -p egui_webview --example minimal
cargo run -p egui_webview --example minimal_instance
```

On the web with [Trunk](https://github.com/trunk-rs/trunk):

```sh
cd crates/egui_webview
trunk serve --example minimal
trunk serve --example minimal_instance
```

`crates/egui_webview_demo` is a bigger demo, with a browser bar, pages in egui windows and a log of the pages' events.


## CEF

CEF forks helper processes from the app's own executable, which is why `egui_webview::init` must run first in `main`.
With CEF enabled it calls `egui_webview::cef_init`, which runs CEF and exits for the spawned helper processes, and for the app process it returns right away.

### Known Limitations

- Pages are rendered on the CPU and uploaded as textures.
- JavaScript dialogs, file pickers, and drag and drop are not yet supported.

### Building

The [`cef`](https://crates.io/crates/cef) crate downloads CEF to `CEF_PATH` (~1 GB) if not already present.

On Windows & macOS, it also builds a small C++ wrapper library which requires CMake and Ninja.
> [!NOTE]
> See cef-rs' README [here](https://github.com/tauri-apps/cef-rs/tree/dev#usage) for more details on building.

### Packaging

* Linux & Windows: the build copies CEF's files next to the binaries in `target/<profile>`.
  A packaged app ships them next to its executable.
* macOS: during development, the app loads CEF from where the build put it.
  A packaged app needs CEF in its app bundle, together with CEF's helper apps.

