//! A page in a [`WebViewInstance`], in an app that starts without CEF.
//!
//! If CEF isn't on this machine, the app shows [`egui_webview::cef_install_ui`] in the page's
//! place. It creates the page once CEF is installed.
//!
//! [`egui_webview::WebView`] does all of this by itself. See the `minimal` example.
//!
//! ```sh
//! cargo run -p egui_webview --features cef-download --example cef_install
//! ```
//!
//! `cargo run` finds the CEF that the build downloaded, so nothing is missing. The README
//! says how to try the download.

mod common;

use egui_webview::{CefStatus, Error, WebViewInstance, WebViewSource};

struct App {
    source: WebViewSource,

    /// The page, once CEF is installed.
    webview: Option<WebViewInstance>,

    /// Why the page couldn't be created, if CEF is there and it still failed.
    error: Option<Error>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(webview) = &mut self.webview {
                webview.ui(ui, ui.available_size());
            } else if let Some(error) = &self.error {
                ui.colored_label(ui.visuals().error_fg_color, error.to_string());
            } else {
                ui.vertical_centered(egui_webview::cef_install_ui);
            }
        });
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        if let Some(webview) = &mut self.webview {
            webview.place(ctx, &output.platform_output.embedded_views);
        } else if self.error.is_none() && egui_webview::cef_status() == CefStatus::Installed {
            // CEF was there from the start, or the install just finished.
            match WebViewInstance::new_cef(ctx, egui::Id::unique("page"), &self.source) {
                Ok(webview) => self.webview = Some(webview),
                Err(error) => self.error = Some(error),
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    common::run(
        "egui_webview cef_install",
        Box::new(|_cc| {
            Ok(Box::new(App {
                source: WebViewSource::Url("https://www.egui.rs".parse()?),
                webview: None,
                error: None,
            }))
        }),
    )
}
