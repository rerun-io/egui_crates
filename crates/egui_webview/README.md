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

The examples run on the web with [Trunk](https://trunkrs.dev/):

```sh
cd crates/egui_webview
trunk serve --example minimal
trunk serve --example minimal_instance
```

`crates/egui_webview_demo` is a bigger demo, with a browser bar, pages in egui windows and a log of the pages' events.

## Backends

* Web: an `<iframe>` below egui's canvas.

The `<iframe>` lies below egui's canvas, and egui clears its rect to transparent so the page shows through.
egui windows and popups above a page paint over it and receive the pointer input there.
