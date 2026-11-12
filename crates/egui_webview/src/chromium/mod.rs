//! A [`Backend`] that renders pages off screen with [CEF](https://github.com/chromiumembedded/cef)
//! (the Chromium Embedded Framework).
//!
//! Each page is a windowless Chromium browser (CEF's off-screen rendering mode). CEF paints the
//! page into a BGRA pixel buffer, which is uploaded to an egui texture, and the widget forwards
//! egui's input events to the browser. Because the page is an ordinary texture, egui can paint on
//! top of it, and it works the same with every egui renderer and on every platform, including
//! Wayland.
//!
//! CEF starts its helper processes from the app's own executable, so apps must call
//! [`crate::run_cef_subprocess`] at the start of `main`.

#![allow(clippy::transmute_ptr_to_ptr)] // Triggered by what the cef-rs `wrap_*` macros expand to.

mod client;
mod input;
mod keys;
mod runtime;

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use cef::{
    BrowserSettings, ImplBrowser as _, ImplBrowserHost as _, ImplFrame as _,
    ImplRunContextMenuCallback as _, WindowInfo,
};
use egui::{
    Color32, Context, EmbeddedViewOutput, EventFilter, FocusDirection, Pos2, Rect, TextureHandle,
    Ui, pos2,
};

use crate::{Backend, Error, WebViewEvent, WebViewSource};
use client::{MenuItem, Shared};

pub use runtime::run_subprocess;

/// Whether `WebViewInstance::new` should use CEF.
///
/// That is the case on platforms without a system webview backend (all but macOS and Windows),
/// and wherever the `EGUI_WEBVIEW_BACKEND` environment variable is `cef`.
pub fn is_default() -> bool {
    cfg!(not(any(target_os = "macos", target_os = "windows")))
        || std::env::var("EGUI_WEBVIEW_BACKEND").is_ok_and(|backend| backend == "cef")
}

/// A [`Backend`] that renders the page in a windowless CEF browser and paints it as an egui
/// texture.
pub struct CefBackend {
    browser: cef::Browser,
    shared: Arc<Shared>,
    input: input::Input,
    texture: Option<TextureHandle>,
    popup_texture: Option<TextureHandle>,

    /// Whether the browser was last told that it is hidden (`was_hidden(1)`), which lets CEF stop
    /// rendering it.
    hidden: bool,

    /// The viewport, and the number of the pass in it (see [`Context::cumulative_pass_nr`]), that
    /// last showed the page.
    shown_in: Option<(egui::ViewportId, u64)>,
}

impl CefBackend {
    /// Creates a windowless browser that loads `source`, and initialises CEF if this is the first
    /// browser in the process.
    ///
    /// # Errors
    /// If CEF can't be initialised, or if it fails to create the browser.
    pub fn new(ctx: &Context, source: &WebViewSource) -> Result<Self, Error> {
        runtime::initialize(ctx)?;

        let shared = Shared::new(ctx);
        let window_info = WindowInfo {
            windowless_rendering_enabled: 1,
            ..Default::default()
        };
        let settings = BrowserSettings {
            windowless_frame_rate: 60,
            // Opaque white like in a browser, for pages that set no background. CEF's default
            // for a windowless browser is transparent.
            background_color: 0xFFFF_FFFF,
            ..Default::default()
        };
        let browser = cef::browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut shared.client()),
            Some(&source_url(source).as_str().into()),
            Some(&settings),
            None,
            None,
        )
        .ok_or_else(|| Error::backend("CEF failed to create a browser"))?;
        // A windowless browser starts out focused. Blur it, so that it has the focus only while
        // egui's focus is on the webview.
        if let Some(host) = browser.host() {
            host.set_focus(0);
        }

        Ok(Self {
            browser,
            shared,
            input: input::Input::default(),
            texture: None,
            popup_texture: None,
            hidden: false,
            shown_in: None,
        })
    }

    /// Updates the browser for this pass and paints the page into `response.rect`.
    fn show(&mut self, ui: &Ui, response: &egui::Response, host: &cef::BrowserHost) {
        let ctx = ui.ctx().clone();
        *self.shared.viewport.lock() = ctx.viewport_id();
        self.shown_in = Some((ctx.viewport_id(), ctx.cumulative_pass_nr()));
        if std::mem::take(&mut self.hidden) {
            host.was_hidden(0);
        }

        let rect = response.rect;
        let ppp = ctx.pixels_per_point();
        let (resized, rescaled) = {
            let mut size = self.shared.size.lock();
            let changed = (size.0 != rect.size(), size.1 != ppp);
            *size = (rect.size(), ppp);
            changed
        };
        if rescaled {
            host.notify_screen_info_changed();
        }
        if resized || rescaled {
            host.was_resized();
        }

        // Keep egui's focus on the webview while its context menu is open, also when the menu is
        // clicked. CEF runs a menu command such as Copy or Paste only in a focused page.
        let menu_open = self.shared.context_menu.lock().is_some();
        let menu_rect = ctx.memory(|mem| mem.area_rect(context_menu_id(response)));
        let pressed_at = ui.input(|i| {
            i.pointer
                .any_pressed()
                .then(|| i.pointer.interact_pos())
                .flatten()
        });
        let keep_focus = menu_open
            && pressed_at.is_none_or(|pos| menu_rect.is_some_and(|menu| menu.contains(pos)));
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
        // menu.
        self.input.handle(ui, response, &self.browser, !menu_open);
        self.paint(ui, rect);
        self.context_menu(ui, response);

        if response.contains_pointer() {
            ctx.set_cursor_icon(*self.shared.cursor.lock());
            if let Some(tooltip) = self.shared.tooltip.lock().clone() {
                response.clone().on_hover_text(tooltip);
            }
        }

        // Enable the IME while an editable element in the page has focus. The text cursor is
        // relative to the view, so it is moved into egui's screen space.
        if response.has_focus() && self.shared.editable.load(Ordering::Relaxed) {
            let cursor = self
                .shared
                .text_cursor
                .lock()
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

    /// Shows the context menu the page asked for.
    ///
    /// Picking an item, pressing Escape or pressing outside the menu closes it and completes its
    /// CEF callback.
    fn context_menu(&self, ui: &Ui, page: &egui::Response) {
        let mut menu = self.shared.context_menu.lock();
        let Some(open) = menu.as_mut() else {
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
        if let Some(id) = picked {
            if let Some(open) = menu.take() {
                open.callback
                    .cont(id, cef::sys::cef_event_flags_t::EVENTFLAG_NONE.into());
            }
        } else if closed && let Some(open) = menu.take() {
            open.callback.cancel();
        }
    }

    /// Uploads new frames to the textures and paints the view and its popup into `rect`.
    fn paint(&mut self, ui: &Ui, rect: Rect) {
        let ctx = ui.ctx();
        self.shared
            .view
            .lock()
            .upload(ctx, "egui_webview", &mut self.texture);
        let popup_rect = *self.shared.popup_rect.lock();
        if popup_rect.is_some() {
            self.shared
                .popup
                .lock()
                .upload(ctx, "egui_webview_popup", &mut self.popup_texture);
        }

        let painter = ui.painter_at(rect);
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0));
        if let Some(texture) = &self.texture {
            // Paint at the texture's own size, so that a frame rendered before a resize isn't
            // stretched to the new rect.
            let size = texture.resolution_vec2() / ctx.pixels_per_point();
            painter.image(
                texture.id(),
                Rect::from_min_size(rect.min, size),
                uv,
                Color32::WHITE,
            );
        }
        if let (Some(popup), Some(texture)) = (popup_rect, &self.popup_texture) {
            painter.image(
                texture.id(),
                popup.translate(rect.min.to_vec2()),
                uv,
                Color32::WHITE,
            );
        }
    }
}

impl Backend for CefBackend {
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        if let Some(frame) = self.browser.main_frame() {
            frame.load_url(Some(&source_url(source).as_str().into()));
        }
        Ok(())
    }

    fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        if let Some(frame) = self.browser.main_frame() {
            frame.execute_java_script(Some(&script.into()), None, 0);
        }
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

    fn ui(&mut self, ui: &mut Ui, response: &egui::Response) {
        if let Some(host) = self.browser.host() {
            self.show(ui, response, &host);
        }
    }

    fn place(&mut self, _ctx: &Context, _output: Option<&EmbeddedViewOutput>) {
        let ctx = &self.shared.ctx;
        // Shown or not, the browser needs CEF's message loop work, so egui must keep running
        // passes.
        ctx.request_repaint_after(runtime::MAX_PUMP_DELAY);

        // This backend isn't an embedded view, so `output` is always `None`. Instead, the page
        // was shown if `ui` ran in the pass that just ended, whose number is one less than the
        // viewport's current pass count.
        let shown = self
            .shown_in
            .is_some_and(|(viewport, pass)| pass + 1 == ctx.cumulative_pass_nr_for(viewport));

        // Let CEF stop rendering a page that isn't shown. `show` tells CEF when it's shown again.
        if !shown && !self.hidden {
            if let Some(host) = self.browser.host() {
                host.was_hidden(1);
            }
            self.hidden = true;
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
        self.shared.took_focus_away.lock().take()
    }

    fn take_events(&mut self) -> Vec<WebViewEvent> {
        std::mem::take(&mut self.shared.events.lock())
    }
}

impl Drop for CefBackend {
    fn drop(&mut self) {
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

/// Rounds `x` to one of CEF's integer coordinates. Out-of-range values saturate, as with `as`.
#[expect(clippy::cast_possible_truncation)]
fn round(x: f32) -> i32 {
    x.round() as i32
}

/// The URL that loads `source`. CEF loads only URLs, so HTML becomes a percent-encoded `data:`
/// URL.
fn source_url(source: &WebViewSource) -> String {
    match source {
        WebViewSource::Url(url) => url.clone(),
        WebViewSource::Html(html) => {
            let mut url = String::from("data:text/html;charset=utf-8,");
            for byte in html.bytes() {
                if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                    url.push(char::from(byte));
                } else {
                    write!(url, "%{byte:02X}").ok();
                }
            }
            url
        }
    }
}
