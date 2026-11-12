//! The smallest app that shows a page with a [`WebViewInstance`].
//!
//! The app creates the page once and keeps it. It shows the page in [`eframe::App::ui`] and
//! places it in [`eframe::App::output_hook`].
//!
//! ```sh
//! cd crates/egui_webview && trunk serve --example minimal_instance
//! ```

mod common;

use egui_webview::{WebViewInstance, WebViewSource};

struct App {
    webview: WebViewInstance,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            self.webview.ui(ui, ui.available_size());
        });
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webview
            .place(ctx, &output.platform_output.embedded_views);
    }
}

fn main() -> eframe::Result {
    common::run(
        "egui_webview minimal_instance",
        Box::new(|cc| {
            let webview = WebViewInstance::new(
                &cc.egui_ctx,
                egui::Id::unique("page"),
                &WebViewSource::Url("https://www.egui.rs".to_owned()),
                cc.embedded_views(),
            )?;
            Ok(Box::new(App { webview }))
        }),
    )
}
