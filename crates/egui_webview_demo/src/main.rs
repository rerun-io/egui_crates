//! A demo of [`egui_webview::WebViewInstance`]: one page in the central panel, and more in egui
//! windows above it, each with a browser bar.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use egui_webview::{WebViewEvent, WebViewInstance, WebViewSource};

/// The page the demo starts with. Its grid of tiles makes scrolling under egui visible.
const PAGE: &str = r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<style>
  body { font-family: system-ui, -apple-system, 'Segoe UI', sans-serif; font-size: 15px; line-height: 1.5; background: #f4f0e8; color: #2b2118; margin: 0; padding: 40px 48px; }
  main { max-width: 600px; display: flex; flex-direction: column; gap: 16px; }
  h1 { margin: 0; font-size: 28px; font-weight: 600; letter-spacing: -0.01em; }
  p { margin: 0; }
  .controls { display: flex; flex-wrap: wrap; gap: 12px; align-items: center; }
  button, input, select { font: inherit; font-size: 14px; padding: 6px 10px; border-radius: 6px; border: 1px solid #b9ae9c; background: #fffdf9; color: #2b2118; }
  button { padding: 6px 14px; cursor: pointer; }
  a { color: #1a5fb4; }
  .hint { color: #6b6052; }
  .tiles { display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 12px; }
  .tiles div { height: 72px; border-radius: 8px; display: flex; align-items: center; justify-content: center; font-size: 14px; }
</style>
</head>
<body>
<main>
  <h1>Hello from the page</h1>
  <p>This is an HTML page in a webview below egui. egui paints its windows, menus and tooltips above it, and the page gets the clicks only where nothing of egui covers it.</p>
  <div class="controls">
    <button onclick="window.ipc.postMessage('Hello from the page!')">Send a message to egui</button>
    <input aria-label="Text to type" placeholder="Type here">
    <select aria-label="Fruit" onchange="window.ipc.postMessage('Picked ' + this.value)">
      <option>Apples</option>
      <option>Bananas</option>
      <option>Cherries</option>
    </select>
  </div>
  <p><a href="https://example.com">Go to example.com</a></p>
  <p class="hint">Scroll the page: it moves under egui's windows.</p>
  <div class="tiles" id="tiles"></div>
</main>
<script>
  const tiles = document.getElementById('tiles');
  for (let i = 0; i < 48; i++) {
    const tile = document.createElement('div');
    tile.textContent = i + 1;
    tile.style.background = `hsl(${(i * 37) % 360}, 70%, 80%)`;
    tiles.appendChild(tile);
  }
</script>
</body>
</html>
"#;

/// The pages the Page menu offers, which new browser windows also cycle through. An empty URL
/// stands for [`PAGE`].
///
/// On the web, many sites refuse to be shown in an `<iframe>`, so these URLs have to allow it.
const SOURCES: [(&str, &str); 3] = [
    ("Demo page", ""),
    ("egui.rs", "https://www.egui.rs"),
    ("example.com", "https://example.com"),
];

const NEW_WINDOW: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N);
const RELOAD: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::R);

fn source(url: &str) -> WebViewSource {
    if url.is_empty() {
        WebViewSource::Html(PAGE.to_owned())
    } else {
        WebViewSource::Url(url.to_owned())
    }
}

/// A webview with a browser bar: navigation buttons, a page menu and a URL field.
struct Browser {
    name: String,
    webview: WebViewInstance,
    url: String,
}

impl Browser {
    fn new(
        ctx: &egui::Context,
        name: String,
        source: &WebViewSource,
        native_views: &eframe::NativeViews,
    ) -> Result<Self, String> {
        let webview = WebViewInstance::new(ctx, egui::Id::unique(&name), source, native_views)
            .map_err(|err| format!("No webview: {err}"))?;
        let url = match source {
            WebViewSource::Url(url) => url.clone(),
            WebViewSource::Html(_) => String::new(),
        };
        Ok(Self { name, webview, url })
    }

    /// The bar of a browser window: everything in one row.
    fn toolbar(&mut self, ui: &mut egui::Ui, log: &mut Vec<String>) {
        ui.horizontal(|ui| {
            self.nav_buttons(ui);
            ui.menu_button("☰", |ui| self.page_menu(ui, log));
            self.url_field(ui, log);
        });
    }

    /// Back, forward and reload: square, and framed only on hover like the menu buttons next
    /// to them.
    fn nav_buttons(&mut self, ui: &mut egui::Ui) {
        let webview = &mut self.webview;
        let side = ui.spacing().interact_size.y;
        let nav = |ui: &mut egui::Ui, glyph: &str, hover: &str| {
            let button = egui::Button::new(glyph)
                .frame_when_inactive(false)
                .min_size(egui::Vec2::splat(side));
            ui.add(button).on_hover_text(hover).clicked()
        };
        if nav(ui, "⏴", "Back") {
            webview.back();
        }
        if nav(ui, "⏵", "Forward") {
            webview.forward();
        }
        if nav(ui, "⟳", "Reload") {
            webview.reload();
        }
    }

    /// The items of the page menu, in the menu bar of the main page and under ☰ in a window.
    fn page_menu(&mut self, ui: &mut egui::Ui, log: &mut Vec<String>) {
        let webview = &mut self.webview;
        for (name, url) in SOURCES {
            if ui.button(name).clicked() {
                report(log, webview.load(&source(url)));
            }
        }
        ui.separator();
        if ui.button("Run script").clicked() {
            report(
                log,
                webview.evaluate_script(
                    "document.body.style.background = `hsl(${Math.random() * 360}, 60%, 85%)`",
                ),
            );
        }
        let reload = egui::Button::new("Reload").shortcut_text(ui.ctx().format_shortcut(&RELOAD));
        if ui.add(reload).clicked() {
            webview.reload();
        }
    }

    /// The URL field, filling the width that "Go" leaves.
    fn url_field(&mut self, ui: &mut egui::Ui, log: &mut Vec<String>) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let go_clicked = ui.button("Go").clicked();
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.url)
                    .hint_text("Type a URL and press Enter")
                    .desired_width(f32::INFINITY),
            );
            let entered =
                field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            if go_clicked || entered {
                report(log, self.webview.load_url(self.url.clone()));
            }
        });
    }

    fn page(&mut self, ui: &mut egui::Ui, log: &mut Vec<String>) {
        let response = self.webview.ui(ui, ui.available_size());
        for event in response.events {
            log.push(format!(
                "{}: {}",
                self.name,
                match event {
                    WebViewEvent::Loading { url } => format!("Loading {}", short_url(&url)),
                    WebViewEvent::Loaded { url } => {
                        // Don't show the URL of a page given as HTML (`about:srcdoc`,
                        // `about:blank` or a `data:` URL) in the URL field.
                        if !url.starts_with("about:") && !url.starts_with("data:") {
                            self.url.clone_from(&url);
                        }
                        format!("Loaded {}", short_url(&url))
                    }
                    WebViewEvent::Message(message) => format!("Message: {message}"),
                    other => format!("{other:?}"),
                }
            ));
        }
    }
}

struct DemoApp {
    main: Result<Browser, String>,
    windows: Vec<(egui::Id, Browser)>,
    opened: usize,
    show_modal: bool,
    show_log: bool,
    log: Vec<String>,
}

impl DemoApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let main = Browser::new(
            &cc.egui_ctx,
            "Main".to_owned(),
            &source(""),
            cc.native_views(),
        );

        Self {
            main,
            windows: Vec::new(),
            opened: 0,
            show_modal: false,
            show_log: true,
            log: Vec::new(),
        }
    }

    fn open_window(&mut self, ctx: &egui::Context, native_views: &eframe::NativeViews) {
        self.opened += 1;
        // The first window shows egui.rs, so that it differs from the main page.
        let (_, url) = SOURCES[self.opened % SOURCES.len()];
        match Browser::new(
            ctx,
            format!("Window {}", self.opened),
            &source(url),
            native_views,
        ) {
            Ok(browser) => self
                .windows
                .push((egui::Id::unique(("browser window", self.opened)), browser)),
            Err(err) => self.log.push(err),
        }
    }
}

impl eframe::App for DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Self {
            main,
            windows,
            show_modal,
            show_log,
            log,
            ..
        } = self;

        let main = match main {
            Ok(main) => main,
            Err(err) => {
                egui::CentralPanel::default().show(ui, |ui| ui.label(err.as_str()));
                return;
            }
        };

        let mut open_window = ui.input_mut(|input| input.consume_shortcut(&NEW_WINDOW));
        if ui.input_mut(|input| input.consume_shortcut(&RELOAD)) {
            main.webview.reload();
        }

        egui::Panel::top("toolbar").show(ui, |ui| {
            // The menu bar takes the whole row, so the browser controls go inside it.
            egui::MenuBar::new().ui(ui, |ui| {
                ui.strong("🌐 egui_webview");
                {
                    ui.menu_button("Windows", |ui| {
                        let new_window = egui::Button::new("New browser window")
                            .shortcut_text(ui.ctx().format_shortcut(&NEW_WINDOW));
                        open_window |= ui.add(new_window).clicked();
                        ui.separator();
                        if ui.button("Close all windows").clicked() {
                            windows.clear();
                        }
                    });
                    ui.menu_button("Page", |ui| main.page_menu(ui, log));
                    ui.menu_button("Demo", |ui| {
                        if ui.button("Show modal…").clicked() {
                            *show_modal = true;
                        }
                        ui.checkbox(show_log, "Show log");
                        ui.menu_button("Theme", |ui| {
                            let current = ui.ctx().theme();
                            let mut theme = current;
                            ui.radio_value(&mut theme, egui::Theme::Dark, "Dark");
                            ui.radio_value(&mut theme, egui::Theme::Light, "Light");
                            if theme != current {
                                ui.ctx().set_theme(theme);
                            }
                        });
                    });
                }
                // The menu bar restyles its buttons; the browser controls look as they do in
                // the windows instead.
                ui.scope(|ui| {
                    ui.reset_style();
                    ui.separator();
                    main.nav_buttons(ui);
                    main.url_field(ui, log);
                });
            });
        });

        if *show_log {
            egui::Panel::bottom("log").resizable(true).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in log.iter() {
                            ui.monospace(line);
                        }
                    });
            });
        }

        egui::CentralPanel::default().show(ui, |ui| main.page(ui, log));

        windows.retain_mut(|(id, browser)| {
            let mut open = true;
            egui::Window::new(browser.name.clone())
                .id(*id)
                .open(&mut open)
                .default_size([480.0, 360.0])
                .show(ui.ctx(), |ui| {
                    browser.toolbar(ui, log);
                    browser.page(ui, log);
                });
            open
        });

        if *show_modal {
            let modal = egui::Modal::new(egui::Id::unique("modal")).show(ui.ctx(), |ui| {
                ui.heading("Close the other windows?");
                ui.label(
                    "While this modal is open, the pages take no clicks: egui's backdrop \
                     covers them.",
                );
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_them = ui.button("Close them").clicked();
                    let cancel = ui.button("Cancel").clicked();
                    (close_them, cancel)
                })
                .inner
            });
            let (close_them, cancel) = modal.inner;
            if close_them {
                windows.clear();
            }
            if close_them || cancel || modal.should_close() {
                *show_modal = false;
            }
        }

        if open_window {
            self.open_window(ui.ctx(), frame.native_views());
        }
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        let views = &output.platform_output.native_views;
        if let Ok(main) = &mut self.main {
            main.webview.place(ctx, views);
        }
        for (_, browser) in &mut self.windows {
            browser.webview.place(ctx, views);
        }
    }
}

/// Shortens `url` for the log. CEF loads a page given as HTML from a `data:` URL that contains
/// the whole document.
fn short_url(url: &str) -> &str {
    if url.starts_with("data:") {
        "data:…"
    } else {
        url
    }
}

fn report(log: &mut Vec<String>, result: Result<(), egui_webview::Error>) {
    if let Err(err) = result {
        log.push(format!("Error: {err}"));
    }
}

// When compiling natively:
#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    // CEF runs its helper processes from this executable. In a helper process, this runs CEF
    // and returns its exit code.
    #[cfg(feature = "cef")]
    if let Some(code) = egui_webview::run_cef_subprocess() {
        std::process::exit(code);
    }

    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    #[cfg_attr(not(target_os = "windows"), expect(unused_mut))]
    let mut options = eframe::NativeOptions {
        // Native pages show through egui's surface where egui clears it, which needs a
        // transparent window.
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_transparent(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    #[cfg(target_os = "windows")]
    egui_webview::wgpu_configuration(&mut options.wgpu_options);

    eframe::run_native(
        "egui_webview demo",
        options,
        Box::new(|cc| Ok(Box::new(DemoApp::new(cc)))),
    )
}

// When compiling to web using trunk:
#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    // Redirect `log` message to `console.log` and friends:
    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let search = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();

    // `?renderer=wgpu` in the URL picks wgpu, otherwise glow.
    let renderer = if search.contains("renderer=wgpu") {
        eframe::Renderer::Wgpu
    } else {
        eframe::Renderer::Glow
    };

    // `&backend=webgl` makes wgpu use WebGL, as it does in a browser without WebGPU.
    let mut wgpu_options = eframe::WgpuConfiguration::default();
    if search.contains("backend=webgl")
        && let eframe::egui_wgpu::WgpuSetup::CreateNew(create_new) = &mut wgpu_options.wgpu_setup
    {
        create_new.instance_descriptor.backends = eframe::wgpu::Backends::GL;
    }

    wasm_bindgen_futures::spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.get_element_by_id("the_canvas_id"))
            .and_then(|element| element.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("No canvas with the id `the_canvas_id`");

        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions {
                    renderer,
                    wgpu_options,
                    ..Default::default()
                },
                Box::new(|cc| Ok(Box::new(DemoApp::new(cc)))),
            )
            .await
            .expect("Failed to start eframe");
    });
}
