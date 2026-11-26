use std::sync::Arc;

use eframe::EmbeddedViews;
use egui::{
    Context, EmbeddedViewOutput, FocusDirection, Id, Key, Modifiers, Response, Theme, Ui, Vec2,
};

use crate::{Backend, Error, PageWidget, Url, WebViewEvent, WebViewSource};

/// A web page shown as an egui widget.
///
/// The instance owns the page, including its history and state, so create it once and keep it.
/// Call [`Self::ui`] in every pass in which the page should be shown, and [`Self::place`] from
/// [`eframe::App::output_hook`], which places the page after the pass, or hides it if
/// [`Self::ui`] wasn't called in that pass.
///
/// Pages live on the thread that runs egui, so this type isn't `Send`.
#[derive(Debug)]
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

    /// The theme last given to [`Backend::set_theme`], or `None` before the first
    /// [`Self::place`].
    theme: Option<Theme>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Gained,
    Lost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusChange {
    pub direction: FocusDirection,
    pub focus: Focus,
}

impl FocusChange {
    pub fn gained(direction: FocusDirection) -> Self {
        Self {
            direction,
            focus: Focus::Gained,
        }
    }
    pub fn lost(direction: FocusDirection) -> Self {
        Self {
            direction,
            focus: Focus::Lost,
        }
    }

    pub fn was_lost(&self) -> bool {
        matches!(self.focus, Focus::Lost)
    }
    pub fn was_gained(&self) -> bool {
        matches!(self.focus, Focus::Gained)
    }
}

/// What [`show_widget`] did in a pass.
pub(crate) struct Shown {
    pub response: Response,

    /// How egui moved the focus to or from the webview in this pass, if it did.
    ///
    /// Tell the page about this with [`WebViewInstance::apply_focus`].
    pub focus_change: Option<FocusChange>,
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

/// The backends that [`WebViewInstance::new`] chooses from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefaultBackend {
    /// The platform's own webview in an embedded view: an `<iframe>` on the web.
    Embedded,

    /// CEF, which renders the page off screen. Requires the `cef` feature.
    Cef,
}

impl DefaultBackend {
    /// The environment variable that overrides [`Self::FOR_TARGET`] at run time.
    const ENV_VAR: &str = "EGUI_WEBVIEW_BACKEND";

    /// The backend of the target platform, chosen at compile time: the `<iframe>` on the web,
    /// and CEF natively.
    const FOR_TARGET: Self = if cfg!(target_arch = "wasm32") {
        Self::Embedded
    } else {
        Self::Cef
    };

    /// The backend that [`Self::ENV_VAR`] names, if it is set to a known one.
    fn from_env() -> Option<Self> {
        let name = std::env::var(Self::ENV_VAR).ok()?;
        if name == "cef" {
            Some(Self::Cef)
        } else {
            log::warn!(
                "egui_webview: ignoring {}={name:?}, expected `cef`",
                Self::ENV_VAR
            );
            None
        }
    }

    /// The backend that [`WebViewInstance::new`] uses.
    fn pick() -> Self {
        Self::from_env().unwrap_or(Self::FOR_TARGET)
    }
}

impl WebViewInstance {
    /// Creates a webview with the platform's default backend, in the root window that
    /// `embedded_views` belongs to.
    ///
    /// The target platform decides the backend at compile time:
    /// - On the web: an `<iframe>` below egui's canvas.
    /// - Natively: CEF, as with `Self::new_cef`. This requires the `cef` feature.
    ///
    /// Setting the `EGUI_WEBVIEW_BACKEND` environment variable to `cef` overrides that choice
    /// at run time.
    ///
    /// The page lives in the root window. Only call [`Self::ui`] in the root viewport, and call
    /// [`Self::place`] from [`eframe::App::output_hook`].
    ///
    /// `id` must be unique in `ctx`: it is the id of both the widget and its embedded view.
    ///
    /// # Errors
    /// [`Error::NoRuntime`] natively without a running [`crate::Runtime`], [`Error::Unsupported`] if no
    /// backend supports this platform or the window of `embedded_views`, and
    /// [`Error::Backend`] if the backend fails to create the page.
    pub fn new(
        ctx: &Context,
        id: impl Into<Id>,
        source: &WebViewSource,
        embedded_views: &EmbeddedViews,
    ) -> Result<Self, Error> {
        #[cfg(not(target_arch = "wasm32"))]
        crate::Runtime::engine()?;
        match DefaultBackend::pick() {
            DefaultBackend::Cef => {
                _ = embedded_views;
                #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
                {
                    Self::new_cef(ctx, id, source)
                }
                #[cfg(not(all(feature = "cef", not(target_arch = "wasm32"))))]
                {
                    _ = (ctx, id, source);
                    Err(Error::Unsupported)
                }
            }
            DefaultBackend::Embedded => {
                #[cfg(target_arch = "wasm32")]
                {
                    let backend = crate::web::IframeBackend::new(ctx, source, embedded_views)?;
                    Ok(Self::with_backend(id, backend))
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    _ = (ctx, id, source, embedded_views);
                    Err(Error::Unsupported)
                }
            }
        }
    }

    /// The widget of the pages that [`Self::new`] creates, if it is the same for all of them.
    ///
    /// A page in an embedded view (the web) needs nothing from the page to reserve its rect, so
    /// this is known before the page exists. A CEF page has nothing to paint
    /// before it exists.
    pub(crate) fn default_widget() -> Option<Arc<dyn PageWidget>> {
        match DefaultBackend::pick() {
            DefaultBackend::Embedded => Some(Arc::new(crate::EmbeddedWidget)),
            DefaultBackend::Cef => None,
        }
    }

    /// Creates a webview that CEF (Chromium) renders off screen into an egui texture.
    ///
    /// This works with every egui renderer (wgpu on Vulkan, Metal or DX12, or glow) on every
    /// native platform, and [`Self::new`] uses it there. It requires the `cef` feature, and a
    /// running [`crate::Runtime`].
    ///
    /// `id` must be unique in `ctx`.
    ///
    /// # Errors
    /// [`Error::NoRuntime`] without a running [`crate::Runtime`], and [`Error::Backend`] if CEF
    /// fails to initialize or to create the browser.
    #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
    pub fn new_cef(
        ctx: &Context,
        id: impl Into<Id>,
        source: &WebViewSource,
    ) -> Result<Self, Error> {
        Self::new_cef_with_options(ctx, id, source, crate::CefOptions::default())
    }

    /// Like [`Self::new_cef`], with `options` for how CEF renders the page.
    ///
    /// To give the pages of a [`crate::WebViews`] options, create it with
    /// [`crate::WebViews::with_factory`] and call this from the factory.
    ///
    /// # Errors
    /// See [`Self::new_cef`].
    #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
    pub fn new_cef_with_options(
        ctx: &Context,
        id: impl Into<Id>,
        source: &WebViewSource,
        options: crate::CefOptions,
    ) -> Result<Self, Error> {
        let backend = crate::chromium::CefBackend::new(ctx, source, options)?;
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
            theme: None,
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
    ///
    /// This also gives the page egui's theme, the first time and whenever it changes.
    pub fn place(&mut self, ctx: &Context, outputs: &[EmbeddedViewOutput]) {
        let theme = ctx.theme();
        if self.theme != Some(theme) {
            self.backend.set_theme(theme);
            self.theme = Some(theme);
        }

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
    pub fn load_url(&mut self, url: Url) -> Result<(), Error> {
        self.load(&WebViewSource::Url(url))
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

    /// Whether [`Self::back`] has a page to go to.
    ///
    /// On the web this is always `true`, because the app can't read the history of the
    /// `<iframe>`.
    pub fn can_go_back(&self) -> bool {
        self.backend.can_go_back()
    }

    /// Whether [`Self::forward`] has a page to go to. See [`Self::can_go_back`] for the web.
    pub fn can_go_forward(&self) -> bool {
        self.backend.can_go_forward()
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

        if ui.memory(|mem| !mem.has_focus(id)) {
            self.blur();
        }
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

    /// Blurs the page.
    pub(crate) fn blur(&mut self) {
        if self.page_focused {
            self.backend.blur();
            self.page_focused = false;
        }
    }

    /// Asks the page whether it took or gave up the keyboard focus by itself since the last
    /// call.
    ///
    /// Pass the result to [`show_widget`] in the next pass.
    pub(crate) fn poll_focus(&mut self) -> Option<FocusChange> {
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
            Some(FocusChange::lost(direction))
        } else if !self.page_focused && self.backend.has_focus() {
            self.page_focused = true;
            Some(FocusChange::gained(FocusDirection::None))
        } else {
            None
        }
    }

    /// Tells the page that egui's focus moved to the webview's widget, or away from it.
    pub(crate) fn apply_focus(&mut self, change: FocusChange) {
        match change.focus {
            Focus::Gained => {
                // Without a direction, such as after `Response::request_focus`, a page that
                // already has the focus keeps its focused element.
                if change.direction != FocusDirection::None || !self.backend.has_focus() {
                    self.backend.focus(change.direction);
                }
                self.page_focused = true;
            }
            Focus::Lost => {
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
/// `pending_focus_change` contains any pending focus change that came from the webview
/// ([`WebViewInstance::poll_focus`]).
pub(crate) fn show_widget(
    ui: &mut Ui,
    id: Id,
    size: Vec2,
    widget: &dyn PageWidget,
    pending_focus_change: Option<FocusChange>,
) -> Shown {
    // Focus doesn't move until the widget is allocated,
    // so any focus blurs need to happen before that.
    if let Some(focus) = pending_focus_change
        && focus.was_lost()
        && ui.memory(|mem| mem.has_focus(id))
    {
        ui.memory_mut(|mem| mem.move_focus(focus.direction));
    }

    let response = widget.allocate(ui, id, size);

    if pending_focus_change.is_some_and(|f| f.was_gained()) {
        response.request_focus();
    }
    if response.gained_focus() && response.focused_via() == Some(FocusDirection::Next) {
        // Consume the tab press when first focusing the webview, so the webview doesn't move focus again
        ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab));
    }
    widget.show(ui, &response);

    // Only tell the webview about focus gains if it doesn't know already
    let focus_change = if response.gained_focus()
        && pending_focus_change.is_none_or(|f| f.focus != Focus::Gained)
    {
        Some(FocusChange::gained(
            response.focused_via().unwrap_or(FocusDirection::None),
        ))
    } else if response.lost_focus() {
        Some(FocusChange::lost(FocusDirection::None))
    } else {
        None
    };
    Shown {
        response,
        focus_change,
    }
}
