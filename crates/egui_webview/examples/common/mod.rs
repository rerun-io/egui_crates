//! Starts an example's app, in a window natively or in the page's canvas on the web.
//!
//! This is the same for every eframe app. It is here so that the examples only show what is
//! about webviews.

/// Runs the app that `create` makes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    // Start the webview runtime right after logging, before anything else. With CEF, Chromium
    // runs its helper processes (renderer, GPU, …) from this same executable. In such a
    // process, this call runs the helper and then exits, so nothing below runs there.
    #[cfg(not(feature = "cef"))]
    let _runtime = egui_webview::Runtime::new();
    // CEF is loaded when the first page needs it. If it isn't on this machine, the webview
    // offers to download it into `cef_dir`. CEF can be downloaded again, so it belongs in
    // the cache folder, in a subfolder of the app's own.
    #[cfg(feature = "cef")]
    let _runtime = egui_webview::Runtime::with_cef_options(egui_webview::CefInitOptions {
        cef_dir: dirs::cache_dir().map(|dir| dir.join("egui_webview_examples").join("cef")),
        ..Default::default()
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        title,
        options,
        Box::new(|cc| {
            install_cef_if_asked(&cc.egui_ctx);
            create(cc)
        }),
    )
}

/// Downloads CEF without asking, if `EGUI_WEBVIEW_CEF_INSTALL` is set and CEF is missing. This
/// is for tests that can't click the webview's download button. A real app asks the user first.
#[cfg(all(feature = "cef-download", not(target_arch = "wasm32")))]
fn install_cef_if_asked(ctx: &egui::Context) {
    if std::env::var_os("EGUI_WEBVIEW_CEF_INSTALL").is_some()
        && egui_webview::cef_status() == egui_webview::CefStatus::NotInstalled
    {
        egui_webview::cef_install(ctx);
    }
}

/// Without the `cef-download` feature, there is nothing to install.
#[cfg(not(any(feature = "cef-download", target_arch = "wasm32")))]
fn install_cef_if_asked(_ctx: &egui::Context) {}

/// Runs the app that `create` makes, in the canvas of `index.html`.
#[cfg(target_arch = "wasm32")]
#[expect(clippy::unnecessary_wraps)] // Same signature as the native `run`.
pub fn run(_title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();
    let _runtime = egui_webview::Runtime::new();

    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("the_canvas_id"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("No canvas with the id `the_canvas_id`");

        eframe::WebRunner::new()
            .start(canvas, eframe::WebOptions::default(), create)
            .await
            .expect("Failed to start eframe");
    });
    Ok(())
}
