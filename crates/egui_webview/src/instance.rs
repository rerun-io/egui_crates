use std::sync::Arc;

use eframe::EmbeddedViews;
use egui::{Context, EmbeddedViewOutput, FocusDirection, Id, Key, Modifiers, Response, Ui, Vec2};

use crate::{Backend, Error, PageWidget, WebViewEvent, WebViewSource};

// The platform's embedded backend, which `WebViewInstance::new` uses unless CEF is the default.
#[cfg(target_os = "macos")]
use crate::macos::WryBackend as PlatformBackend;
#[cfg(target_arch = "wasm32")]
use crate::web::IframeBackend as PlatformBackend;
#[cfg(target_os = "windows")]
use crate::win32::WryBackend as PlatformBackend;

/// A web page shown as an egui widget.
///
/// The instance owns the page, including its history and state, so create it once and keep it.
/// Call [`Self::ui`] in every pass in which the page should be shown, and [`Self::place`] from
/// [`eframe::App::output_hook`], which places the page after the pass, or hides it if
/// [`Self::ui`] wasn't called in that pass.
///
/// Pages live on the thread that runs egui, so this type isn't `Send`.
pub struct WebViewInstance {
    id: Id,
    backend: Box<dyn Backend>,

    /// The backend's [`Backend::widget`].
    widget: Arc<dyn PageWidget>,

    /// Whether the page has the keyboard focus, as far as this instance knows.
    ///
    /// Set when egui's focus moves to the webview, or when the backend reports that the page
    /// took the focus itself. Cleared when the focus leaves the page or the webview.
    page_focused: bool,
}

/// A keyboard focus change that the page made itself, rather than egui.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PageFocus {
    /// The page took the focus, for example because it was clicked.
    Took,

    /// Tab or Shift-Tab moved the focus out of the page, toward the next or previous widget.
    Left(FocusDirection),
}

/// What happened to egui's focus on the webview's widget in a pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusChange {
    /// The widget got the focus. Tab is [`FocusDirection::Next`], Shift-Tab is
    /// [`FocusDirection::Previous`], and anything else, such as a click, is
    /// [`FocusDirection::None`].
    Gained(FocusDirection),

    /// The widget lost the focus.
    Lost,
}

/// What [`show_widget`] did in a pass.
pub(crate) struct Shown {
    pub response: Response,

    /// Tell the page about this with [`WebViewInstance::apply_focus`].
    pub focus_change: Option<FocusChange>,
}

impl std::fmt::Debug for WebViewInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebViewInstance")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The result of [`WebViewInstance::ui`] and [`crate::WebView::show`].
#[derive(Debug)]
pub struct WebViewResponse {
    /// The events from the page since the previous call.
    pub events: Vec<WebViewEvent>,

    /// The response for the page's rect. [`Response::has_focus`] is `true` while the page has
    /// the keyboard focus.
    pub response: Response,
}

impl WebViewInstance {
    /// Creates a webview with the platform's default backend, in the root window that
    /// `embedded_views` belongs to.
    ///
    /// - On the web: an `<iframe>` below egui's canvas.
    /// - On macOS: a `WKWebView` below egui's surface. It needs eframe's wgpu renderer.
    /// - On Windows: a `WebView2` below egui's surface. It needs a transparent window, and eframe's
    ///   wgpu renderer configured with `wgpu_configuration`.
    /// - On other platforms, or when the `EGUI_WEBVIEW_BACKEND` environment variable is `cef`: CEF,
    ///   as with `Self::new_cef`. This requires the `cef` feature.
    ///
    /// The page lives in the root window. Only call [`Self::ui`] in the root viewport, and call
    /// [`Self::place`] from [`eframe::App::output_hook`].
    ///
    /// `id` must be unique in `ctx`: it is the id of both the widget and its embedded view.
    ///
    /// # Errors
    /// [`Error::Unsupported`] if no backend supports this platform or the window of
    /// `embedded_views`, and [`Error::Backend`] if the backend fails to create the page.
    pub fn new(
        ctx: &Context,
        id: impl Into<Id>,
        source: &WebViewSource,
        embedded_views: &EmbeddedViews,
    ) -> Result<Self, Error> {
        #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
        if crate::chromium::is_default() {
            _ = embedded_views;
            return Self::new_cef(ctx, id, source);
        }

        #[cfg(any(target_arch = "wasm32", target_os = "macos", target_os = "windows"))]
        {
            let backend = PlatformBackend::new(ctx, source, embedded_views)?;
            Ok(Self::with_backend(id, backend))
        }
        #[cfg(not(any(target_arch = "wasm32", target_os = "macos", target_os = "windows")))]
        {
            _ = (ctx, id, source, embedded_views);
            Err(Error::Unsupported)
        }
    }

    /// The widget of the pages that [`Self::new`] creates, if it is the same for all of them.
    ///
    /// A page in an embedded view (web, macOS, Windows) needs nothing from the page to reserve
    /// its rect, so this is known before the page exists. A CEF page has nothing to paint
    /// before it exists.
    pub(crate) fn default_widget() -> Option<Arc<dyn PageWidget>> {
        #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
        if crate::chromium::is_default() {
            return None;
        }
        if cfg!(any(
            target_arch = "wasm32",
            target_os = "macos",
            target_os = "windows"
        )) {
            Some(Arc::new(crate::EmbeddedWidget))
        } else {
            None
        }
    }

    /// Creates a webview that CEF (Chromium) renders off screen into an egui texture.
    ///
    /// This works with every egui renderer (wgpu on Vulkan, Metal or DX12, or glow) on every
    /// native platform. [`Self::new`] uses it on platforms without a system webview, such as
    /// Linux, and when the `EGUI_WEBVIEW_BACKEND` environment variable is `cef`. It requires the
    /// `cef` feature, and a call to [`crate::run_cef_subprocess`] at the start of `main`.
    ///
    /// `id` must be unique in `ctx`.
    ///
    /// # Errors
    /// [`Error::Backend`] if CEF fails to initialize or to create the browser.
    #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
    pub fn new_cef(
        ctx: &Context,
        id: impl Into<Id>,
        source: &WebViewSource,
    ) -> Result<Self, Error> {
        let backend = crate::chromium::CefBackend::new(ctx, source)?;
        Ok(Self::with_backend(id, backend))
    }

    /// Creates a webview that shows its page through `backend`.
    ///
    /// `id` must be unique in the [`Context`].
    pub fn with_backend(id: impl Into<Id>, backend: impl Backend + 'static) -> Self {
        Self {
            id: id.into(),
            widget: backend.widget(),
            backend: Box::new(backend),
            page_focused: false,
        }
    }

    /// Places the page where the pass that produced `outputs` put its embedded view, or hides it
    /// if the view isn't in `outputs`.
    ///
    /// Call this from [`eframe::App::output_hook`] with
    /// [`egui::PlatformOutput::embedded_views`], for every webview the app owns.
    ///
    /// Call it for a backend that renders into a texture too. It has no view to place, but it
    /// uses the call to stop rendering a page that isn't shown.
    pub fn place(&mut self, ctx: &Context, outputs: &[EmbeddedViewOutput]) {
        // egui also reports a view that only touches the clip rect, with nothing to see.
        let output = outputs
            .iter()
            .find(|output| output.id == self.id && output.visible.is_positive());
        self.backend.place(ctx, output);
    }

    /// The id that this webview was created with.
    pub fn id(&self) -> Id {
        self.id
    }

    /// Loads `source`, replacing the current page.
    ///
    /// # Errors
    /// If the page can't start loading, for example because the engine can't parse the URL.
    /// Errors that occur while the page loads aren't reported.
    pub fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        self.backend.load(source)
    }

    /// Loads the page at `url`.
    ///
    /// # Errors
    /// See [`Self::load`].
    pub fn load_url(&mut self, url: impl Into<String>) -> Result<(), Error> {
        self.load(&WebViewSource::Url(url.into()))
    }

    /// Runs `script` in the page.
    ///
    /// # Errors
    /// If the script can't be passed to the page, for example on the web for a cross-origin
    /// page. Most engines run the script asynchronously, so exceptions it throws aren't reported.
    pub fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        self.backend.evaluate_script(script)
    }

    /// Reloads the current page.
    ///
    /// A page given as [`WebViewSource::Html`] is loaded again from that HTML.
    ///
    /// On the web, this loads the last [`WebViewSource`] again and so discards any navigation
    /// since then, because only a same-origin page lets the app call `location.reload()`.
    pub fn reload(&mut self) {
        self.backend.reload();
    }

    /// Navigates back in the webview's session history.
    ///
    /// On the web, the `<iframe>` shares the joint session history of the browser tab, so when
    /// the page has nothing to go back to, this navigates the app's own document back instead.
    /// For a cross-origin page it does nothing, because the app can't run scripts in it.
    pub fn back(&mut self) {
        self.backend.back();
    }

    /// Navigates forward in the webview's session history. See [`Self::back`] for the web.
    pub fn forward(&mut self) {
        self.backend.forward();
    }

    /// Shows the page in a rect of `size`, and returns the page's events since the last call.
    ///
    /// The page takes part in egui's keyboard focus like a widget. Tab moves the focus to the
    /// first element in the page's tab order, and Shift-Tab to the last. Tab from the page's last
    /// element moves the focus on to the next egui widget, and Shift-Tab from its first element to
    /// the previous one. The [`Response`] has egui's focus while the page has the keyboard focus.
    ///
    /// On the web, the app can't focus an element of a cross-origin page. For such a page, the
    /// focus enters the page with the user's next Tab or Shift-Tab, which the browser handles.
    ///
    /// Only call this in the root viewport. Other viewports aren't supported yet.
    pub fn ui(&mut self, ui: &mut Ui, size: Vec2) -> WebViewResponse {
        let id = self.id;

        self.blur_unless(ui.memory(|mem| mem.has_focus(id)));
        let page_focus = self.poll_focus();

        let shown = show_widget(ui, id, size, &*self.widget, page_focus);
        if let Some(change) = shown.focus_change {
            self.apply_focus(change);
        }
        let events = self.take_events();
        WebViewResponse {
            events,
            response: shown.response,
        }
    }

    /// The half of the page that runs in egui's pass.
    pub(crate) fn widget(&self) -> Arc<dyn PageWidget> {
        Arc::clone(&self.widget)
    }

    /// Blurs the page if it thinks it has the focus while its widget doesn't.
    ///
    /// egui clears the focus of a widget that wasn't shown in a pass without ever reporting
    /// `lost_focus` for it.
    pub(crate) fn blur_unless(&mut self, widget_focused: bool) {
        if self.page_focused && !widget_focused {
            self.backend.blur();
            self.page_focused = false;
        }
    }

    /// Asks the page whether it took or gave up the keyboard focus by itself since the last
    /// call.
    ///
    /// Pass the result to [`show_widget`] in the next pass.
    pub(crate) fn poll_focus(&mut self) -> Option<PageFocus> {
        if let Some(direction) = self.backend.poll_focus_left() {
            // Ignore it if the page didn't have the focus. On the web, for example, the
            // browser's Tab through the document around egui's canvas also passes the focus
            // edges of the `<iframe>`, without entering the page.
            if !std::mem::take(&mut self.page_focused) {
                return None;
            }
            // Blur now rather than on `lost_focus`. For Shift-Tab, egui only moves its focus in
            // the next pass, and until then a page that keeps the focus after Tab left it
            // (`WebView2`) would be polled as having taken the focus back.
            self.backend.blur();
            Some(PageFocus::Left(direction))
        } else if !self.page_focused && self.backend.has_focus() {
            self.page_focused = true;
            Some(PageFocus::Took)
        } else {
            None
        }
    }

    /// Tells the page that egui's focus moved to the webview's widget, or away from it.
    pub(crate) fn apply_focus(&mut self, change: FocusChange) {
        match change {
            FocusChange::Gained(direction) => {
                // Without a direction, such as after `Response::request_focus`, a page that
                // already has the focus keeps its focused element.
                if direction != FocusDirection::None || !self.backend.has_focus() {
                    self.backend.focus(direction);
                }
                self.page_focused = true;
            }
            FocusChange::Lost => {
                self.backend.blur();
                self.page_focused = false;
            }
        }
    }

    /// The events from the page since the last call, oldest first.
    pub(crate) fn take_events(&mut self) -> Vec<WebViewEvent> {
        self.backend.take_events()
    }
}

/// Runs the half of a webview that needs no page. It allocates the rect, moves egui's focus,
/// and shows `widget`.
///
/// `page_focus` is what the page did to the keyboard focus by itself, from
/// [`WebViewInstance::poll_focus`].
pub(crate) fn show_widget(
    ui: &mut Ui,
    id: Id,
    size: Vec2,
    widget: &dyn PageWidget,
    page_focus: Option<PageFocus>,
) -> Shown {
    // Do this before the widget is created: `Memory::move_focus` only moves the focus away from
    // a widget when that widget registers for focus in `Ui::interact` later in the pass.
    if let Some(PageFocus::Left(direction)) = page_focus
        && ui.memory(|mem| mem.has_focus(id))
    {
        ui.memory_mut(|mem| mem.move_focus(direction));
    }

    let response = widget.allocate(ui, id, size);

    // This focus work has to run before `PageWidget::show`. A page that egui paints reads
    // egui's focus and input there.
    if page_focus == Some(PageFocus::Took) {
        response.request_focus();
    }
    if response.gained_focus() && response.focused_via() == Some(FocusDirection::Next) {
        // egui used this Tab to move its focus to the page. A page that gets egui's key events
        // must not get this one, or it moves its own focus a second time.
        ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab));
    }
    widget.show(ui, &response);

    let focus_change = if response.gained_focus() && page_focus != Some(PageFocus::Took) {
        // Tab enters the page at its first element, and Shift-Tab at its last. The widget has
        // the focus here, so there is a direction. For a click it is `FocusDirection::None`.
        let direction = response.focused_via().unwrap_or(FocusDirection::None);
        Some(FocusChange::Gained(direction))
    } else if response.lost_focus() {
        Some(FocusChange::Lost)
    } else {
        None
    };
    Shown {
        response,
        focus_change,
    }
}
