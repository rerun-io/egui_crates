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

## Backends

* Web: an `<iframe>` below egui's canvas.

The `<iframe>` lies below egui's canvas, and egui clears its rect to transparent so the page shows through.
egui windows and popups above a page paint over it and receive the pointer input there.
