use std::sync::Arc;

use eframe::EmbeddedViews;
use egui::{Context, EmbeddedViewOutput, FocusDirection, Id, Key, Modifiers, Response, Ui, Vec2};

use crate::{Backend, Error, PageWidget, WebViewEvent, WebViewSource};

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

    /// Whether the widget has egui's focus at the end of the call.
    pub focused: bool,

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

    /// Creates a webview that shows its page through `backend`.
    ///
    /// `id` must be unique in the [`Context`].
    pub fn with_backend(id: impl Into<Id>, backend: impl Backend + 'static) -> Self {
        Self {
            id: id.into(),
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
        let output = outputs.iter().find(|output| output.id == self.id);
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
        let widget = self.backend.widget();

        self.blur_unless(ui.memory(|mem| mem.has_focus(id)));
        let page_focus = self.poll_focus();

        let shown = show_widget(ui, id, size, &*widget, page_focus);
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
        self.backend.widget()
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
        let page_focus = poll_page_focus(&mut self.page_focused, &mut *self.backend);
        if matches!(page_focus, Some(PageFocus::Left(_))) {
            // Blur now rather than on `lost_focus`. For Shift-Tab, egui only moves its focus in
            // the next pass, and until then a page that keeps the focus after Tab left it
            // (`WebView2`) would be polled as having taken the focus back.
            self.backend.blur();
        }
        page_focus
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
    // Whether Tab was pressed in this pass, matched the way egui matches it for focus
    // navigation.
    let tab = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    modifiers,
                    ..
                } if !modifiers.any()
            )
        })
    });

    let response = widget.allocate(ui, id, size);

    // This focus work has to run before `PageWidget::show`. A page that egui paints reads
    // egui's focus and input there.
    if page_focus == Some(PageFocus::Took) {
        response.request_focus();
    }
    if response.clicked_elsewhere() && ui.memory(|mem| mem.has_focus(id)) {
        response.surrender_focus();
    }
    if tab && response.gained_focus() {
        // egui used this Tab to move its focus to the page. A page that gets egui's key events
        // must not get this one, or it moves its own focus a second time.
        ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Tab));
    }
    widget.show(ui, &response);

    let focus_change = if response.gained_focus() && page_focus != Some(PageFocus::Took) {
        // Tab enters the page at its first element, and Shift-Tab at its last.
        Some(FocusChange::Gained(response.focused_via()))
    } else if response.lost_focus() {
        Some(FocusChange::Lost)
    } else {
        None
    };
    let focused = response.has_focus();
    Shown {
        response,
        focused,
        focus_change,
    }
}

/// Polls `backend` for a focus change that the page made itself since the last call, and updates
/// `page_focused` to match.
fn poll_page_focus(page_focused: &mut bool, backend: &mut dyn Backend) -> Option<PageFocus> {
    if let Some(direction) = backend.poll_focus_left() {
        // Ignore it if the page didn't have the focus. On the web, for example, the browser's Tab
        // through the document around egui's canvas also passes the focus edges of the
        // `<iframe>`, without entering the page.
        std::mem::take(page_focused).then_some(PageFocus::Left(direction))
    } else if !*page_focused && backend.has_focus() {
        *page_focused = true;
        Some(PageFocus::Took)
    } else {
        None
    }
}
