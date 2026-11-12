//! The smallest app that shows a page with [`WebView`].
//!
//! The app keeps a [`WebViews`], which owns the pages, and updates it in
//! [`eframe::App::output_hook`]. The code that shows the page only needs a [`egui::Ui`].
//!
//! ```sh
//! cd crates/egui_webview && trunk serve --example minimal
//! ```

mod common;

use egui_webview::{WebView, WebViews};

struct App {
    webviews: WebViews,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            WebView::new(egui::Id::unique("page"))
                .url("https://www.egui.rs")
                .show(ui);
        });
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webviews.update(ctx, output);
    }
}

fn main() -> eframe::Result {
    common::run(
        "egui_webview minimal",
        Box::new(|cc| {
            let webviews = WebViews::new(&cc.egui_ctx, cc.embedded_views());
            Ok(Box::new(App { webviews }))
        }),
    )
}
