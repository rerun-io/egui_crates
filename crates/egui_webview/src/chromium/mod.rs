//! A [`Backend`] that renders pages off screen with [CEF](https://github.com/chromiumembedded/cef)
//! (the Chromium Embedded Framework).
//!
//! Each page is a windowless Chromium browser (CEF's off-screen rendering mode). CEF paints the
//! page into a BGRA pixel buffer, which is uploaded to an egui texture, and egui's input events
//! are forwarded to the browser. Because the page is an ordinary texture, egui can paint on
//! top of it, and it works the same with every egui renderer and on every platform, including
//! Wayland.
//!
//! The backend has two halves. The widget half is [`Context`]. It runs in egui's pass, paints the
//! page, and writes down the pass's input. The page half is [`CefBackend`]. It owns the browser,
//! and sends it that input after the pass.
//!
//! CEF starts its helper processes from the app's own executable, so apps must create a
//! [`crate::Runtime`] at the start of `main`.

#![allow(clippy::transmute_ptr_to_ptr)] // Triggered by what the cef-rs `wrap_*` macros expand to.

mod context;
mod handlers;
mod input;
mod keys;
mod layer;
mod runtime;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use cef::{
    BrowserSettings, ImplBrowser as _, ImplBrowserHost as _, ImplFrame as _,
    ImplRunContextMenuCallback as _, WindowInfo,
};
use egui::{
    Color32, EmbeddedViewOutput, EventFilter, FocusDirection, Id, Pos2, Rect, Sense, Theme, Ui,
    Vec2, ViewportId, pos2,
};

use crate::{Backend, Error, PageWidget, WebViewEvent, WebViewSource};
use context::Context;
use handlers::MenuItem;

pub use runtime::{CefInitOptions, CefProcess, init, run_subprocess, shutdown};

/// How CEF renders a page. Pass it to [`crate::WebViewInstance::new_cef_with_options`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CefOptions {
    /// The maximum number of frames per second that CEF paints outside a resize.
    ///
    /// The default, `None`, follows the current monitor's refresh rate and falls back to 60 FPS
    /// when it is unknown. Use `Some(fps)` for a fixed rate. The minimum is 1 FPS.
    ///
    /// While the page is resized, CEF paints as fast as it can so the page follows its widget.
    /// It returns to the normal rate 100 ms after the last resize.
    pub frame_rate: Option<u32>,

    /// The color behind the page. It shows until a page has loaded, and where a page sets no
    /// background of its own.
    ///
    /// The default is white.
    ///
    /// CEF takes an opaque background or none at all. With [`Color32::TRANSPARENT`], egui
    /// shows through such a page. The alpha of every other color is ignored.
    pub background_color: Color32,
}

impl Default for CefOptions {
    fn default() -> Self {
        Self {
            frame_rate: None,
            background_color: Color32::WHITE,
        }
    }
}

/// CEF takes a whole number of frames per second.
fn page_frame_rate(ctx: &egui::Context, frame_rate: Option<u32>) -> i32 {
    if let Some(fps) = frame_rate {
        return i32::try_from(fps).unwrap_or(i32::MAX).max(1);
    }
    ctx.input_for(ViewportId::ROOT, |i| {
        #[expect(clippy::cast_possible_truncation)]
        i.viewport()
            .monitor_refresh_rate
            .filter(|hz| hz.is_finite() && 0.0 < *hz)
            .map_or(60, |hz| (hz.round() as i32).max(1))
    })
}

/// `color` as CEF's ARGB color. CEF takes a background that is opaque or fully transparent, so
/// every color that isn't fully transparent becomes opaque.
fn cef_color(color: Color32) -> u32 {
    if color.a() == 0 {
        return 0;
    }
    let [r, g, b, _] = color.to_srgba_unmultiplied();
    u32::from_be_bytes([0xFF, r, g, b])
}

/// The target of the trace logs about the frames of a page: `painted <height in pixels>
/// <kept|stale>` for each frame that CEF paints, and `shown <the widget's height> <the frame's
/// height>`, in points, for each pass that shows the page.
pub(crate) const FRAMES_LOG_TARGET: &str = "egui_webview::frames";

/// The frame rate CEF gets while its page is resized: as fast as it can paint.
///
/// CEF needs a few frames to follow each resize, so the page's frame rate decides how far the
/// page lags behind its widget. See the `resize_bench` example.
const RESIZE_FRAME_RATE: i32 = 1000;

/// How long CEF keeps [`RESIZE_FRAME_RATE`] after a resize.
const RESIZE_FRAME_RATE_DURATION: Duration = Duration::from_millis(100);

/// How many passes of input are kept for the backend. More than that only pile up if the app
/// doesn't call [`crate::WebViewInstance::place`].
const MAX_WAITING_BATCHES: usize = 64;

/// A [`Backend`] that renders the page in a windowless CEF browser and paints it as an egui
/// texture.
pub struct CefBackend {
    browser: cef::Browser,
    context: Arc<Context>,

    /// What happened in the page, sent by the CEF handlers.
    events: mpsc::Receiver<WebViewEvent>,
    input: input::Input,

    /// Whether the browser was last told that it is hidden (`was_hidden(1)`), which lets CEF stop
    /// rendering it.
    hidden: bool,

    /// The view's size in points and the pixels per point that the browser was last told.
    told_size: Option<(Vec2, f32)>,

    frame_rate: FrameRate,

    // Drop after the browser and its callbacks have released their CEF references.
    _engine: crate::runtime::Engine,
}

/// The frame rate of a [`CefBackend`]'s browser.
#[derive(Debug)]
struct FrameRate {
    /// The fixed rate requested by the caller, if any.
    requested: Option<u32>,

    /// The frame rate the browser was last told, including the resize boost.
    told: i32,

    /// When the browser was last resized, while it still paints at [`RESIZE_FRAME_RATE`].
    resized_at: Option<Instant>,
}

impl FrameRate {
    /// Tells `host` the frame rate it should have now, if it changed.
    fn update(&mut self, ctx: &egui::Context, host: &cef::BrowserHost) {
        let mut frame_rate = page_frame_rate(ctx, self.requested);
        if let Some(resized_at) = self.resized_at {
            let left = RESIZE_FRAME_RATE_DURATION.saturating_sub(resized_at.elapsed());
            if left.is_zero() {
                self.resized_at = None;
            } else {
                frame_rate = frame_rate.max(RESIZE_FRAME_RATE);
                ctx.request_repaint_after(left);
            }
        }
        if self.told != frame_rate {
            host.set_windowless_frame_rate(frame_rate);
            self.told = frame_rate;
        }
    }
}

impl std::fmt::Debug for CefBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CefBackend")
            .field("context", &self.context)
            .field("events", &self.events)
            .field("input", &self.input)
            .field("hidden", &self.hidden)
            .field("told_size", &self.told_size)
            .field("frame_rate", &self.frame_rate)
            .finish_non_exhaustive()
    }
}

impl CefBackend {
    /// Creates a windowless browser that loads `source`, and initializes CEF if this is the first
    /// browser in the process.
    ///
    /// # Errors
    /// If CEF can't be initialized, or if it fails to create the browser.
    pub fn new(
        ctx: &egui::Context,
        source: &WebViewSource,
        options: CefOptions,
    ) -> Result<Self, Error> {
        let engine = runtime::initialize(ctx)?;

        let (context, events) = Context::new(ctx);
        let frame_rate = page_frame_rate(ctx, options.frame_rate);
        let background_color = cef_color(options.background_color);
        let url = source_url(source);
        let for_client = Arc::clone(&context);
        // CEF creates a browser on its UI thread. That can be another thread than this one, so
        // only plain values go over, and CEF's structs are made there.
        let browser = runtime::wait_on_ui_thread(move || {
            let window_info = WindowInfo {
                windowless_rendering_enabled: 1,
                ..Default::default()
            };
            let settings = BrowserSettings {
                windowless_frame_rate: frame_rate,
                background_color,
                ..Default::default()
            };
            cef::browser_host_create_browser_sync(
                Some(&window_info),
                Some(&mut handlers::client(&for_client)),
                Some(&url.as_str().into()),
                Some(&settings),
                None,
                None,
            )
        })
        .flatten()
        .ok_or_else(|| Error::backend("CEF failed to create a browser"))?;
        // A windowless browser starts out focused. Blur it, so that it has the focus only while
        // egui's focus is on the webview.
        if let Some(host) = browser.host() {
            host.set_focus(0);
        }

        Ok(Self {
            browser,
            context,
            events,
            input: input::Input::default(),
            hidden: false,
            told_size: None,
            frame_rate: FrameRate {
                requested: options.frame_rate,
                told: frame_rate,
                resized_at: None,
            },
            _engine: engine,
        })
    }

    /// The browser's main frame. CEF has none once the browser is closed.
    fn main_frame(&self) -> Result<cef::Frame, Error> {
        self.browser
            .main_frame()
            .ok_or_else(|| Error::backend("the CEF browser has no main frame"))
    }
}

/// The half of a CEF page that runs in egui's pass.
impl PageWidget for Context {
    fn allocate(&self, ui: &mut Ui, id: Id, size: Vec2) -> egui::Response {
        let (_, rect) = ui.allocate_space(size);
        ui.interact(rect, id, Sense::click_and_drag())
    }

    /// Paints the page into `response.rect`, and writes down this pass's input for the backend.
    fn show(&self, ui: &mut Ui, response: &egui::Response) {
        let ctx = ui.ctx().clone();
        let rect = response.rect;

        // A page that is scrolled out of view counts as not shown, so CEF stops rendering it.
        let visible = ui.is_rect_visible(rect);
        let pass = ctx.cumulative_pass_nr_for(ViewportId::ROOT);
        let (menu_open, popup) = {
            let mut state = self.state.lock();
            if visible {
                state.shown_pass = Some(pass);
            }
            // CEF asks for this size when it renders. The backend tells CEF after the pass that
            // it changed. CEF takes whole points, so a change of less isn't one.
            state.size = (rect.size().round(), ctx.pixels_per_point());
            (state.context_menu.is_some(), state.popup_placement)
        };

        if visible {
            self.paint(ui, rect);
        }
        if !response.enabled() {
            response.surrender_focus();
            let mut state = self.state.lock();
            state.inbox.batches.clear();
            state.inbox.release_pointer = true;
            if let Some(menu) = state.context_menu.take() {
                state.inbox.menu_done = Some((menu, None));
            }
            return;
        }

        // Keep egui's focus on the webview while its context menu is open, also when the menu is
        // clicked. CEF runs a menu command such as Copy or Paste only in a focused page.
        let keep_focus = menu_open && {
            let menu_rect = ctx.memory(|mem| mem.area_rect(context_menu_id(response)));
            let pressed_at = ui.input(|i| {
                i.pointer
                    .any_pressed()
                    .then(|| i.pointer.interact_pos())
                    .flatten()
            });
            pressed_at.is_none_or(|pos| menu_rect.is_some_and(|menu| menu.contains(pos)))
        };
        if keep_focus && !response.has_focus() {
            response.request_focus();
        }
        if response.has_focus() {
            // The page handles Tab, the arrow keys and Escape, so they mustn't move egui's
            // focus. CEF reports Tab past the page's last element through `on_take_focus`.
            // While the context menu is open, they mustn't move it either.
            ui.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    response.id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }

        // While the context menu is open, keys don't go to the page, so Escape only closes the
        // menu. That has to be decided now. By the time the backend sends the input, the menu
        // is closed.
        let batch = input::Batch::record(ui, response, popup, !menu_open);
        if !batch.is_empty() {
            let mut state = self.state.lock();
            let batches = &mut state.inbox.batches;
            batches.push(batch);
            let waiting = batches.len();
            if MAX_WAITING_BATCHES < waiting {
                batches.drain(..waiting - MAX_WAITING_BATCHES);
            }
        }

        self.show_context_menu(ui, response);

        let state = self.state.lock();
        if response.contains_pointer() {
            ctx.set_cursor_icon(state.cursor);
            if let Some(tooltip) = &state.tooltip {
                response.clone().on_hover_text(tooltip.as_str());
            }
        }

        // Enable the IME while an editable element in the page has focus. The text cursor is
        // relative to the view, so it is moved into egui's screen space.
        if response.has_focus() && state.editable() {
            let cursor = state
                .text_cursor
                .map_or(rect, |cursor| cursor.translate(rect.min.to_vec2()));
            ctx.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect: cursor,
                    purpose: egui::IMEPurpose::Normal,
                    should_interrupt_composition: false,
                });
            });
        }
    }
}

impl Context {
    /// Shows the context menu the page asked for.
    ///
    /// Picking an item, pressing Escape or pressing outside the menu closes it. The backend
    /// completes its CEF callback after the pass.
    fn show_context_menu(&self, ui: &Ui, page: &egui::Response) {
        let mut state = self.state.lock();
        let Some(open) = state.context_menu.as_mut() else {
            return;
        };
        let first_time = !std::mem::replace(&mut open.shown, true);

        let mut picked = None;
        let area = egui::Area::new(context_menu_id(page))
            .kind(egui::UiKind::Menu)
            .order(egui::Order::Foreground)
            .fixed_pos(page.rect.min + open.pos.to_vec2())
            .sizing_pass(first_time)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    egui::containers::menu::menu_style(ui.style_mut());
                    for item in &open.items {
                        match item {
                            MenuItem::Separator => {
                                ui.separator();
                            }
                            MenuItem::Command {
                                id,
                                label,
                                enabled,
                                checked,
                            } => {
                                let button = egui::Button::selectable(*checked, label.as_str());
                                if ui.add_enabled(*enabled, button).clicked() {
                                    picked = Some(*id);
                                }
                            }
                        }
                    }
                });
            })
            .response;

        let closed = ui.input(|i| {
            i.key_pressed(egui::Key::Escape)
                || (i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|pos| !area.rect.contains(pos)))
        });
        if (picked.is_some() || closed)
            && let Some(open) = state.context_menu.take()
        {
            // Taking the menu out hides it right away. The backend tells CEF what was picked.
            state.inbox.menu_done = Some((open, picked));
        }
    }

    /// Uploads new frames to the textures and paints the view and its popup into `rect`.
    fn paint(&self, ui: &Ui, rect: Rect) {
        let ctx = ui.ctx();
        let painter = ui.painter_at(rect);
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));

        let mut state = self.state.lock();
        if let Some(texture) = state.view.upload(ctx, "egui_webview") {
            // Paint at the texture's own size, so that a frame rendered before a resize isn't
            // stretched to the new rect. While the view grows, what is behind the widget shows
            // in the part that the frame doesn't cover yet.
            let size = texture.resolution_vec2() / ctx.pixels_per_point();
            // The `resize_bench` example reads this.
            log::trace!(
                target: FRAMES_LOG_TARGET,
                "shown {} {}",
                rect.height().round(),
                size.y
            );
            painter.image(
                texture.id(),
                Rect::from_min_size(rect.min, size),
                uv,
                Color32::WHITE,
            );
        }
        if let Some(popup) = state.popup_placement
            && let Some(texture) = state.popup.upload(ctx, "egui_webview_popup")
        {
            painter.image(
                texture.id(),
                popup.rect.translate(rect.min.to_vec2()),
                uv,
                Color32::WHITE,
            );
        }
    }
}

impl Backend for CefBackend {
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        self.main_frame()?
            .load_url(Some(&source_url(source).as_str().into()));
        Ok(())
    }

    fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        self.main_frame()?
            .execute_java_script(Some(&script.into()), None, 0);
        Ok(())
    }

    fn reload(&mut self) {
        self.browser.reload();
    }

    fn back(&mut self) {
        self.browser.go_back();
    }

    fn forward(&mut self) {
        self.browser.go_forward();
    }

    fn can_go_back(&self) -> bool {
        self.browser.can_go_back() == 1
    }

    fn can_go_forward(&self) -> bool {
        self.browser.can_go_forward() == 1
    }

    fn widget(&self) -> Arc<dyn PageWidget> {
        Arc::<Context>::clone(&self.context)
    }

    fn set_theme(&mut self, theme: Theme) {
        // A windowless browser ignores CEF's color scheme setting, so emulate the media feature
        // through DevTools instead. The emulation stays when the page navigates.
        let scheme = match theme {
            Theme::Dark => "dark",
            Theme::Light => "light",
        };
        let message = format!(
            r#"{{"id":1,"method":"Emulation.setEmulatedMedia","params":{{"features":[{{"name":"prefers-color-scheme","value":"{scheme}"}}]}}}}"#
        );
        if let Some(host) = self.browser.host() {
            // CEF only takes DevTools messages on its UI thread.
            runtime::on_ui_thread(move || {
                if host.send_dev_tools_message(Some(message.as_bytes())) == 0 {
                    log::warn!("egui_webview: CEF didn't take the color scheme");
                }
            });
        }
    }

    /// Tells the browser what its widget saw in the pass that just ended.
    fn place(&mut self, ctx: &egui::Context, _output: Option<&EmbeddedViewOutput>) {
        // Where egui pumps CEF, the browser needs passes to make progress, shown or not.
        if !runtime::THREADED {
            ctx.request_repaint_after(runtime::MAX_PUMP_DELAY);
        }

        let Some(host) = self.browser.host() else {
            return;
        };
        // CEF can run the handlers from inside the calls below, so the state can't stay locked.
        let (inbox, shown_pass, size) = {
            let mut state = self.context.state.lock();
            (
                std::mem::take(&mut state.inbox),
                state.shown_pass,
                state.size,
            )
        };

        // The page was shown if its widget ran, and was on screen, in the pass that just ended.
        let shown =
            shown_pass.is_some_and(|pass| pass + 1 == ctx.cumulative_pass_nr_for(ViewportId::ROOT));

        if shown {
            if std::mem::take(&mut self.hidden) {
                host.was_hidden(0);
            }
            if self.told_size != Some(size) {
                let rescaled = self.told_size.is_some_and(|(_, scale)| scale != size.1);
                if rescaled {
                    host.notify_screen_info_changed();
                }
                host.was_resized();
                self.told_size = Some(size);
                self.frame_rate.resized_at = Some(Instant::now());
            }
        }

        self.frame_rate.update(ctx, &host);

        if inbox.release_pointer {
            self.input.release(&host);
        }
        for batch in &inbox.batches {
            self.input.replay(&self.browser, batch);
        }
        if let Some((menu, picked)) = inbox.menu_done {
            match picked {
                Some(id) => menu
                    .callback
                    .cont(id, cef::sys::cef_event_flags_t::EVENTFLAG_NONE.into()),
                None => menu.callback.cancel(),
            }
        }

        if !shown {
            self.input.release(&host);
            // Let CEF stop rendering the page. It is told when the page is back.
            if !std::mem::replace(&mut self.hidden, true) {
                host.was_hidden(1);
            }
        }
    }

    fn focus(&mut self, direction: FocusDirection) {
        if let Some(host) = self.browser.host() {
            host.set_focus(1);
        }
        if let Some(script) = crate::backend::focus_script(direction) {
            self.evaluate_script(&script).ok();
        }
    }

    fn blur(&mut self) {
        if let Some(host) = self.browser.host() {
            host.set_focus(0);
        }
    }

    fn poll_focus_left(&mut self) -> Option<FocusDirection> {
        self.context.state.lock().took_focus_away.take()
    }

    fn take_events(&mut self) -> Vec<WebViewEvent> {
        self.events.try_iter().collect()
    }
}

impl Drop for CefBackend {
    fn drop(&mut self) {
        // The widget can outlive the browser. Release its CEF callbacks before shutdown.
        let (open, done) = {
            let mut state = self.context.state.lock();
            (state.context_menu.take(), state.inbox.menu_done.take())
        };
        for menu in open.into_iter().chain(done.map(|(menu, _)| menu)) {
            menu.callback.cancel();
        }
        if let Some(host) = self.browser.host() {
            // Force the close, which skips the page's `beforeunload` handlers.
            host.close_browser(1);
        }
    }
}

/// The [`egui::Area`] id of the context menu of the page whose response is `page`.
fn context_menu_id(page: &egui::Response) -> egui::Id {
    page.id.with("context_menu")
}

/// The URL that loads `source`. CEF loads only URLs, so HTML becomes a base64 `data:` URL.
fn source_url(source: &WebViewSource) -> String {
    use base64::Engine as _;

    match source {
        WebViewSource::Url(url) => url.as_str().to_owned(),
        WebViewSource::Html(html) => {
            let mut url = String::from("data:text/html;charset=utf-8;base64,");
            base64::engine::general_purpose::STANDARD.encode_string(html, &mut url);
            url
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_frame_rate_follows_the_monitor_unless_fixed() {
        let ctx = egui::Context::default();
        let automatic = CefOptions::default().frame_rate;
        for (hz, expected) in [(Some(143.98), 144), (Some(75.0), 75), (None, 60)] {
            ctx.input_mut(|i| {
                i.raw
                    .viewports
                    .entry(ViewportId::ROOT)
                    .or_default()
                    .monitor_refresh_rate = hz;
            });
            assert_eq!(page_frame_rate(&ctx, automatic), expected);
            assert_eq!(page_frame_rate(&ctx, Some(30)), 30);
        }
    }

    #[test]
    fn disabled_page_releases_capture_and_ignores_input() {
        let ctx = egui::Context::default();
        let (page, _events) = Context::new(&ctx);
        let input = egui::RawInput {
            events: vec![egui::Event::PointerMoved(pos2(10.0, 10.0))],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            ui.disable();
            let response = page.allocate(ui, ui.make_persistent_id("page"), Vec2::splat(100.0));
            page.show(ui, &response);
        });
        output.textures_delta.clear();

        let state = page.state.lock();
        assert!(state.inbox.batches.is_empty());
        assert!(state.inbox.release_pointer);
        assert!(state.shown_pass.is_some());
    }

    #[test]
    fn cef_colors() {
        assert_eq!(cef_color(Color32::TRANSPARENT), 0);
        assert_eq!(cef_color(Color32::from_rgb(1, 2, 3)), 0xFF01_0203);
        let translucent = Color32::from_rgba_unmultiplied(255, 0, 0, 1);
        assert_eq!(cef_color(translucent) >> 24, 0xFF);
    }

    #[test]
    fn source_urls() -> Result<(), crate::url::ParseError> {
        let url = "https://example.com/a?c=d";
        assert_eq!(source_url(&WebViewSource::Url(url.parse()?)), url);
        assert_eq!(
            source_url(&WebViewSource::Html("<a>é%#".to_owned())),
            "data:text/html;charset=utf-8;base64,PGE+w6klIw=="
        );
        assert_eq!(
            source_url(&WebViewSource::Html(String::new())),
            "data:text/html;charset=utf-8;base64,"
        );
        Ok(())
    }
}
