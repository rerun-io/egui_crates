//! Starts an example's app, in a window natively or in the page's canvas on the web.
//!
//! This is the same for every eframe app. It is here so that the examples only show what is
//! about webviews.

/// Runs the app that `create` makes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    // With CEF, its helper processes run from this executable. A helper process ends here.
    egui_webview::init();

    let viewport = egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]);
    // On Windows, a page only shows through egui's surface in a transparent window.
    #[cfg(all(feature = "wry", target_os = "windows"))]
    let viewport = viewport.with_transparent(true);

    #[cfg_attr(not(all(feature = "wry", target_os = "windows")), expect(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport,
        // The pages on macOS and Windows need the wgpu renderer.
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    // On Windows, egui has to draw above the page's child window.
    #[cfg(all(feature = "wry", target_os = "windows"))]
    egui_webview::platform::win32::apply_wgpu_configuration(&mut options.wgpu_options);

    eframe::run_native(title, options, create)
}

/// Runs the app that `create` makes, in the canvas of `index.html`.
#[cfg(target_arch = "wasm32")]
#[expect(clippy::unnecessary_wraps)] // Same signature as the native `run`.
pub fn run(_title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();
    egui_webview::init();

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
