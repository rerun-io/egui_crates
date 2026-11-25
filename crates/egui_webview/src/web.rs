//! The web backend: each page is an `<iframe>` below egui's canvas.
//!
//! egui clears the visible rect of each webview to transparent ([`egui::Shape::Clear`]), so the
//! page shows through the canvas, and the egui layers above the webview, such as windows and
//! popups, blend over it.
//!
//! Each `<iframe>` is inside a `<div>` container that clips it to the visible part of its `Ui`.
//! The containers go in the element eframe keeps for views below the canvas
//! ([`eframe::EmbeddedViews::container`]), which precedes the canvas in the document and is painted
//! below it.
//!
//! Because the canvas is on top, eframe routes the pointer itself: it lets the pointer through
//! to a page wherever egui doesn't take it ([`eframe::EmbeddedViews`]).
//!
//! An invisible [`FocusEdge`] before and after each `<iframe>` lets the browser's own Tab move
//! the keyboard focus into and out of the page. This works for pages from any origin.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Context, EmbeddedViewOutput, FocusDirection, Theme};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{HtmlCanvasElement, HtmlElement, HtmlIFrameElement, KeyboardEvent, MessageEvent};

use crate::{Backend, Error, WebViewEvent, WebViewSource, reported_url, warn};

/// The URL that the browser reports for a [`WebViewSource::Html`] page.
const HTML_URL: &str = "about:srcdoc";

/// A script inserted at the start of every [`WebViewSource::Html`] page.
///
/// Such a page runs in a sandboxed `<iframe>` with an opaque origin, so the app can't access it
/// directly. The prelude gives the page `window.ipc.postMessage`, as the other backends do, and
/// runs the scripts that [`IframeBackend::evaluate_script`] posts to it.
const HTML_PRELUDE: &str = r#"<script>
window.ipc = { postMessage: (message) => window.parent.postMessage(message, "*") };
window.addEventListener("message", (event) => {
    const script = event.source === window.parent && event.data && event.data.__egui_webview_eval;
    if (typeof script === "string") {
        // `window.eval` runs the script in the global scope, like `evaluateJavaScript` on the
        // other backends. A plain `eval(script)` would run it inside this listener.
        window.eval(script);
    }
});
</script>"#;

/// An invisible, focusable `<div>` just before or after the `<iframe>` in the sequential focus
/// navigation order.
///
/// The browser's own Tab from the edge before the `<iframe>` focuses the page's first element,
/// and Shift-Tab from the edge after it focuses the page's last element. This is the only way
/// to focus a particular element of a cross-origin page.
///
/// When Tab or Shift-Tab leaves the page, the adjacent edge receives the focus, and its `focus`
/// event records which way the focus left.
#[derive(Debug)]
struct FocusEdge {
    element: HtmlElement,
    _on_focus: Closure<dyn FnMut()>,
    _on_keydown: Closure<dyn FnMut(KeyboardEvent)>,
}

impl FocusEdge {
    /// Creates the edge that the focus reaches when it leaves the page in `direction`:
    /// [`FocusDirection::Previous`] for the edge before the `<iframe>`, and
    /// [`FocusDirection::Next`] for the edge after it.
    ///
    /// `left` and `parked` are shared with [`IframeBackend`].
    fn new(
        document: &web_sys::Document,
        ctx: &Context,
        direction: FocusDirection,
        left: &Rc<Cell<Option<FocusDirection>>>,
        parked: &Rc<Cell<bool>>,
    ) -> Result<Self, Error> {
        let element: HtmlElement = document
            .create_element("div")
            .map_err(js_error)?
            .unchecked_into();
        element.set_tab_index(0);
        element
            .set_attribute(
                "style",
                "position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none;",
            )
            .ok();

        let on_focus = {
            let (ctx, left, parked) = (ctx.clone(), Rc::clone(left), Rc::clone(parked));
            Closure::<dyn FnMut()>::new(move || {
                parked.set(false);
                left.set(Some(direction));
                ctx.request_repaint();
            })
        };
        // Say the user tabs from an egui widget to a cross-origin page. The focus is parked on
        // the edge before the page. If they press Shift-Tab now, the browser would move the
        // focus to some other HTML element. So cancel the key, and report that the focus left
        // the page, so egui moves its focus back. The edge after the page does the same for Tab.
        // If the focus got here by leaving the page, it isn't parked, and the key goes to the
        // browser.
        let on_keydown = {
            let (ctx, left, parked) = (ctx.clone(), Rc::clone(left), Rc::clone(parked));
            Closure::<dyn FnMut(KeyboardEvent)>::new(move |event: KeyboardEvent| {
                let away = event.key() == "Tab"
                    && event.shift_key() == (direction == FocusDirection::Previous);
                if away && parked.get() {
                    event.prevent_default();
                    left.set(Some(direction));
                    ctx.request_repaint();
                }
            })
        };
        element
            .add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref())
            .map_err(js_error)?;
        element
            .add_event_listener_with_callback("keydown", on_keydown.as_ref().unchecked_ref())
            .map_err(js_error)?;
        Ok(Self {
            element,
            _on_focus: on_focus,
            _on_keydown: on_keydown,
        })
    }
}

/// The inline styles of an [`IframeBackend`]'s container and `<iframe>`.
#[derive(Debug, PartialEq)]
struct Styles {
    container: String,
    iframe: String,
}

/// Shows a page in an `<iframe>` below the canvas. See the module docs.
#[derive(Debug)]
pub struct IframeBackend {
    canvas: HtmlCanvasElement,

    container: HtmlElement,
    iframe: HtmlIFrameElement,
    source: WebViewSource,

    /// Whether the `<iframe>` fired its `load` event for the current source, so scripts can
    /// run in its page.
    loaded: Rc<Cell<bool>>,

    /// Scripts from [`Backend::evaluate_script`] that wait for the `load` event.
    pending_scripts: Rc<RefCell<Vec<String>>>,

    events: Rc<RefCell<Vec<WebViewEvent>>>,

    /// The inline styles last set on the container and the `<iframe>`, or `None` while hidden.
    /// Used to skip redundant style writes.
    last_style: Option<Styles>,

    /// The theme from [`Backend::set_theme`]. The `<iframe>` gets it as its `color-scheme`.
    theme: Theme,

    /// See [`Backend::set_document_start_script`]. Only a [`WebViewSource::Html`] page runs it.
    document_start_script: Option<String>,

    /// The [`FocusEdge`]s before and after the `<iframe>`, in that order.
    focus_edges: [FocusEdge; 2],

    /// The direction in which the keyboard focus left the page, set by a [`FocusEdge`].
    ///
    /// A Tab or Shift-Tab away from the page while the focus is parked on an edge counts too.
    focus_left: Rc<Cell<Option<FocusDirection>>>,

    /// Whether [`Backend::focus`] parked the focus on an edge, so the user's next Tab carries it
    /// into the page.
    ///
    /// An edge can also have the focus because the focus just left the page. Then it isn't
    /// parked.
    focus_parked: Rc<Cell<bool>>,

    _on_load: Closure<dyn FnMut()>,
    on_message: Closure<dyn FnMut(MessageEvent)>,
    on_window_blur: Closure<dyn FnMut()>,
}

impl IframeBackend {
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        embedded_views: &eframe::EmbeddedViews,
    ) -> Result<Self, Error> {
        let canvas = embedded_views
            .canvas()
            .ok_or_else(|| Error::backend("The embedded views aren't attached to a canvas"))?
            .clone();
        let window = web_sys::window().ok_or_else(|| Error::backend("No window"))?;
        let document = window
            .document()
            .ok_or_else(|| Error::backend("No document"))?;
        let parent = embedded_views.container().map_err(js_error)?;

        let container: HtmlElement = document
            .create_element("div")
            .map_err(js_error)?
            .unchecked_into();
        // We don't know the page's rect yet, so hide it until the first `place`. This also
        // matches `last_style`, which starts as `None`.
        container.set_attribute("style", "display: none;").ok();
        let iframe: HtmlIFrameElement = document
            .create_element("iframe")
            .map_err(js_error)?
            .unchecked_into();

        let focus_left = Rc::new(Cell::new(None));
        let focus_parked = Rc::new(Cell::new(false));
        let [before, after] = [FocusDirection::Previous, FocusDirection::Next]
            .map(|direction| FocusEdge::new(&document, ctx, direction, &focus_left, &focus_parked));
        let (before, after) = (before?, after?);
        container.append_child(&before.element).map_err(js_error)?;
        container.append_child(&iframe).map_err(js_error)?;
        container.append_child(&after.element).map_err(js_error)?;

        let loaded = Rc::new(Cell::new(false));
        let pending_scripts = Rc::new(RefCell::new(Vec::<String>::new()));
        let events = Rc::new(RefCell::new(Vec::new()));

        let on_load = {
            let ctx = ctx.clone();
            let iframe = iframe.clone();
            let loaded = Rc::clone(&loaded);
            let pending_scripts = Rc::clone(&pending_scripts);
            let events = Rc::clone(&events);
            Closure::<dyn FnMut()>::new(move || {
                loaded.set(true);
                for script in pending_scripts.take() {
                    warn(run_script(&iframe, &script));
                }
                if let Some(url) = reported_url(&url(&iframe)) {
                    events.borrow_mut().push(WebViewEvent::Loaded { url });
                }
                ctx.request_repaint();
            })
        };
        iframe.set_onload(Some(on_load.as_ref().unchecked_ref()));

        // Any frame in the document can post to the window, so accept only messages from this
        // `<iframe>`'s page.
        let on_message = {
            let ctx = ctx.clone();
            let iframe = iframe.clone();
            let events = Rc::clone(&events);
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let from_page = event
                    .source()
                    .zip(iframe.content_window())
                    .is_some_and(|(source, page)| JsValue::from(source) == JsValue::from(page));
                if let Some(text) = event.data().as_string().filter(|_| from_page) {
                    events.borrow_mut().push(WebViewEvent::Message(text));
                    ctx.request_repaint();
                }
            })
        };
        window
            .add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref())
            .map_err(js_error)?;

        // The window gets a `blur` event when the focus moves into an `<iframe>`, including by
        // the browser's Tab from a focus edge. egui gets no event for that, so request a pass,
        // in which the webview polls `has_focus`.
        let on_window_blur = {
            let ctx = ctx.clone();
            Closure::<dyn FnMut()>::new(move || ctx.request_repaint())
        };
        window
            .add_event_listener_with_callback("blur", on_window_blur.as_ref().unchecked_ref())
            .map_err(js_error)?;

        let mut this = Self {
            canvas,
            container,
            iframe,
            source: source.clone(),
            loaded,
            pending_scripts,
            events,
            last_style: None,
            theme: Theme::Light,
            document_start_script: None,
            focus_edges: [before, after],
            focus_left,
            focus_parked,
            _on_load: on_load,
            on_message,
            on_window_blur,
        };
        // Set the source before the `<iframe>` is in the document, so that it doesn't load
        // `about:blank` first.
        this.load(source)?;
        parent.append_child(&this.container).map_err(js_error)?;
        Ok(this)
    }

    /// The inline styles of the container and the `<iframe>` for `output`.
    ///
    /// `output` is in ui points. `scale` is how many CSS pixels one ui point is, which
    /// [`css_pixels_per_point`] measures.
    fn style(output: &EmbeddedViewOutput, scale: f32, theme: Theme) -> Styles {
        let rect = output.rect * scale;
        let visible = output.visible * scale;
        let z_index = output.z_index;

        // eframe keeps the top left corner of the container's parent on the canvas's, so the
        // container is positioned relative to the canvas, and scrolls with it.
        let container = format!(
            "position: absolute; left: {}px; top: {}px; width: {}px; height: {}px; \
             overflow: hidden; pointer-events: none; z-index: {};",
            visible.min.x,
            visible.min.y,
            visible.width(),
            visible.height(),
            z_index,
        );

        // The `<iframe>` is positioned relative to the container, which clips it to `visible`.
        // An `<iframe>` is transparent where its page paints no background, so give it the
        // background of a browser tab, which also shows while the page loads. `Canvas` is that
        // color for the `<iframe>`'s `color-scheme`, so it is dark in the dark theme.
        let offset = rect.min - visible.min;
        let iframe = format!(
            "position: absolute; left: {}px; top: {}px; width: {}px; height: {}px; \
             border: none; color-scheme: {}; background: Canvas; pointer-events: auto;",
            offset.x,
            offset.y,
            rect.width(),
            rect.height(),
            color_scheme(theme),
        );
        Styles { container, iframe }
    }

    /// Whether the page has the keyboard focus.
    ///
    /// With `or_edge`, also whether one of its [`Self::focus_edges`] has it,
    /// as one does while the focus is parked there,
    /// or for a moment when Tab moves the focus out of it.
    fn is_focused(&self, or_edge: bool) -> bool {
        let Some(active) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
        else {
            return false;
        };
        let active = JsValue::from(active);
        active == JsValue::from(&self.iframe)
            || (or_edge
                && self
                    .focus_edges
                    .iter()
                    .any(|edge| active == JsValue::from(&edge.element)))
    }
}

impl Backend for IframeBackend {
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        self.loaded.set(false);
        // Scripts queued for the previous page must not run in the new one.
        self.pending_scripts.borrow_mut().clear();
        let iframe = &self.iframe;
        match source {
            WebViewSource::Html(html) => {
                // Without `allow-same-origin`, the sandbox gives the page an opaque origin, so
                // it can't access the app's document. A change to `sandbox` takes effect at the
                // next navigation, so set it before `srcdoc`.
                iframe
                    .set_attribute("sandbox", "allow-scripts allow-forms allow-popups")
                    .map_err(js_error)?;
                iframe.set_srcdoc(&with_prelude(html, self.document_start_script.as_deref()));
            }
            WebViewSource::Url(url) => {
                iframe.remove_attribute("sandbox").map_err(js_error)?;
                // `srcdoc` takes precedence over `src`, and removing `srcdoc` navigates to
                // `src`. Set `src` first, or the `<iframe>` loads its old `src` (`about:blank`
                // if it had none) and then the URL.
                iframe.set_src(url.as_str());
                if iframe.has_attribute("srcdoc") {
                    iframe.remove_attribute("srcdoc").map_err(js_error)?;
                }
            }
        }
        self.source = source.clone();
        let url = match source {
            WebViewSource::Html(_) => reported_url(HTML_URL),
            WebViewSource::Url(url) => Some(url.clone()),
        };
        if let Some(url) = url {
            self.events.borrow_mut().push(WebViewEvent::Loading { url });
        }
        Ok(())
    }

    fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        if self.loaded.get() {
            run_script(&self.iframe, script)
        } else {
            self.pending_scripts.borrow_mut().push(script.to_owned());
            Ok(())
        }
    }

    fn reload(&mut self) {
        // A cross-origin page doesn't let the app call `location.reload()`, so load the source
        // again. This discards any navigation within the page since the last `load`.
        warn(self.load(&self.source.clone()));
    }

    fn back(&mut self) {
        warn(self.evaluate_script("history.back()"));
    }

    fn forward(&mut self) {
        warn(self.evaluate_script("history.forward()"));
    }

    fn place(&mut self, ctx: &Context, output: Option<&EmbeddedViewOutput>) {
        // A hidden page needs no scale, so it doesn't measure the canvas.
        let style = output.map(|output| {
            let scale = css_pixels_per_point(ctx, &self.canvas.get_bounding_client_rect());
            Self::style(output, scale, self.theme)
        });
        if style == self.last_style {
            return;
        }
        match &style {
            None => {
                self.container.set_attribute("style", "display: none;").ok();
            }
            Some(Styles { container, iframe }) => {
                self.container.set_attribute("style", container).ok();
                self.iframe.set_attribute("style", iframe).ok();
            }
        }
        self.last_style = style;
    }

    fn set_document_start_script(&mut self, script: Option<&str>) {
        // The app can't run a script at the start of a document that the browser loads from a
        // URL. An HTML page gets the script in its prelude.
        self.document_start_script = script.map(str::to_owned);
        // Give it to an HTML page that still loads too. Setting `srcdoc` again starts the load
        // over, which costs little this early, and the browser reports one `load`.
        if !self.loaded.get()
            && let WebViewSource::Html(html) = &self.source
        {
            self.iframe.set_srcdoc(&with_prelude(html, script));
        }
    }

    fn set_theme(&mut self, theme: Theme) {
        // The page's `prefers-color-scheme` follows the `<iframe>`'s `color-scheme`. `place`
        // includes it when it writes the whole style. Set it here too, so that a hidden page
        // gets it before `place` runs again.
        self.theme = theme;
        warn(
            self.iframe
                .style()
                .set_property("color-scheme", color_scheme(theme))
                .map_err(js_error),
        );
    }

    fn focus(&mut self, direction: FocusDirection) {
        // From an edge, the browser's next Tab (or Shift-Tab) moves the focus into the page,
        // whatever its origin.
        let [before, after] = &self.focus_edges;
        match direction {
            FocusDirection::Next => before.element.focus(),
            FocusDirection::Previous => after.element.focus(),
            _ => self.iframe.focus(),
        }
        .ok();
        // If an edge's `focus` event fired, it was for this call, not for the focus leaving
        // the page.
        self.focus_left.set(None);
        self.focus_parked.set(matches!(
            direction,
            FocusDirection::Next | FocusDirection::Previous
        ));
        // In a page that runs the app's scripts, focus the element directly instead of waiting
        // for that Tab. Don't queue the script while the page loads, because the focus may
        // have moved elsewhere by the time it runs.
        if let Some(script) = crate::backend::focus_script(direction)
            && self.loaded.get()
            && let Err(err) = run_script(&self.iframe, &script)
        {
            log::debug!("egui_webview: can't focus an element of the page: {err}");
        }
    }

    fn blur(&mut self) {
        self.focus_parked.set(false);
        if self.is_focused(true) {
            // As in eframe, focusing the canvas must not scroll the document it's embedded in.
            let options = web_sys::FocusOptions::new();
            options.set_prevent_scroll(true);
            self.canvas.focus_with_options(&options).ok();
        }
    }

    fn has_focus(&self) -> bool {
        self.is_focused(false)
    }

    fn poll_focus_left(&mut self) -> Option<FocusDirection> {
        self.focus_left.take()
    }

    fn take_events(&mut self) -> Vec<WebViewEvent> {
        self.events.take()
    }
}

impl Drop for IframeBackend {
    fn drop(&mut self) {
        // Removing the container while it holds the focus would leave the focus on `<body>`,
        // so give it to the canvas first.
        self.blur();
        if let Some(window) = web_sys::window() {
            window
                .remove_event_listener_with_callback(
                    "message",
                    self.on_message.as_ref().unchecked_ref(),
                )
                .ok();
            window
                .remove_event_listener_with_callback(
                    "blur",
                    self.on_window_blur.as_ref().unchecked_ref(),
                )
                .ok();
        }
        self.iframe.set_onload(None);
        self.container.remove();
    }
}

/// How many CSS pixels one ui point takes up on the page.
///
/// Usually this is the zoom factor. But after the canvas is resized, egui keeps the old size for
/// a moment, and the browser stretches what egui drew to the new size. Dividing the two widths
/// gives the factor that matches what is on screen, so the page stays lined up with egui.
fn css_pixels_per_point(ctx: &Context, canvas: &web_sys::DomRect) -> f32 {
    #[expect(clippy::cast_possible_truncation)]
    let scale = (canvas.width() as f32) / ctx.viewport_rect().width();
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        ctx.zoom_factor()
    }
}

/// Runs `script` in the loaded page of `iframe`.
///
/// # Errors
/// If the `<iframe>` has no page, the page is cross-origin, or the script throws in a
/// same-origin page.
fn run_script(iframe: &HtmlIFrameElement, script: &str) -> Result<(), Error> {
    let page = iframe
        .content_window()
        .ok_or_else(|| Error::backend("The iframe has no page"))?;
    if iframe.has_attribute("srcdoc") {
        // A `srcdoc` page has an opaque origin, so post the script to the prelude, which
        // evaluates it.
        let message = js_sys::Object::new();
        js_sys::Reflect::set(&message, &"__egui_webview_eval".into(), &script.into())
            .map_err(js_error)?;
        page.post_message(&message, "*").map_err(js_error)
    } else {
        // Reading `eval` from a cross-origin window throws a `SecurityError`.
        let eval: js_sys::Function = js_sys::Reflect::get(&page, &"eval".into())
            .map_err(js_error)?
            .dyn_into()
            .map_err(js_error)?;
        eval.call1(&page, &script.into())
            .map(drop)
            .map_err(js_error)
    }
}

/// The URL of the page in `iframe`.
///
/// Falls back to the `src` attribute for a cross-origin page, whose location the app can't read.
fn url(iframe: &HtmlIFrameElement) -> String {
    if iframe.has_attribute("srcdoc") {
        return HTML_URL.to_owned();
    }
    iframe
        .content_window()
        .and_then(|page| page.location().href().ok())
        .unwrap_or_else(|| iframe.src())
}

/// Inserts [`HTML_PRELUDE`] and `document_start_script` at the start of `html`, after the
/// doctype, which must come first.
///
/// `document_start_script` must not contain `</script>`.
fn with_prelude(html: &str, document_start_script: Option<&str>) -> String {
    let trimmed = html.trim_start();
    let doctype_end = trimmed
        .get(..9)
        .filter(|start| start.eq_ignore_ascii_case("<!doctype"))
        .and_then(|_| trimmed.find('>'))
        .map_or(0, |end| end + 1);
    let (doctype, rest) = trimmed.split_at(doctype_end);
    let start_script = document_start_script
        .map(|script| format!("<script>{script}</script>"))
        .unwrap_or_default();
    format!("{doctype}{HTML_PRELUDE}{start_script}{rest}")
}

/// The CSS `color-scheme` of `theme`.
fn color_scheme(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
    }
}

fn js_error(err: impl Into<JsValue>) -> Error {
    let err = err.into();
    let message = err
        .dyn_ref::<js_sys::Error>()
        .map_or_else(|| format!("{err:?}"), |err| String::from(err.message()));
    Error::backend(message)
}
