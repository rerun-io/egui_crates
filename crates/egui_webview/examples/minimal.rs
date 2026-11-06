//! The smallest app that shows a page: one [`WebViewInstance`], shown in [`eframe::App::ui`]
//! and placed in [`eframe::App::output_hook`].
//!
//! ```sh
//! cargo run -p egui_webview --example minimal
//! ```

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
            .place(ctx, &output.platform_output.native_views);
    }
}

fn main() -> eframe::Result {
    // CEF runs its helper processes from this executable.
    #[cfg(feature = "cef")]
    if let Some(code) = egui_webview::run_cef_subprocess() {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
        // A native page shows through egui's surface, which needs a transparent window.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_transparent(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "egui_webview minimal",
        options,
        Box::new(|cc| {
            let webview = WebViewInstance::new(
                &cc.egui_ctx,
                egui::Id::unique("page"),
                &WebViewSource::Url("https://www.egui.rs".to_owned()),
                cc.native_views(),
            )?;
            Ok(Box::new(App { webview }))
        }),
    )
}
