//! Starts an example's app, in a window natively or in the page's canvas on the web.
//!
//! This is the same for every eframe app. It is here so that the examples only show what is
//! about webviews.

/// Runs the app that `create` makes.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(title, options, create)
}

/// Runs the app that `create` makes, in the canvas of `index.html`.
#[cfg(target_arch = "wasm32")]
#[expect(clippy::unnecessary_wraps)] // Same signature as the native `run`.
pub fn run(_title: &str, create: eframe::AppCreator<'static>) -> eframe::Result {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

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
