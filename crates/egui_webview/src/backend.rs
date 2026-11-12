use std::sync::Arc;

use egui::{Context, EmbeddedViewOutput, FocusDirection, Id, Response, Ui, Vec2};

use crate::{Error, WebViewEvent, WebViewSource};

/// The half of a page that runs in egui's pass.
///
/// A [`Backend`] hands it out with [`Backend::widget`]. It is `Send + Sync`, so widget code can
/// hold it even though the page itself isn't `Send`.
///
/// The webview calls [`Self::allocate`] first, then moves egui's keyboard focus, then calls
/// [`Self::show`].
#[expect(unused_variables)] // The default implementations ignore their arguments.
pub trait PageWidget: Send + Sync {
    /// Allocates the page's rect in `ui`.
    ///
    /// `id` is the webview's id. Use it for the widget, so that egui's keyboard focus works.
    fn allocate(&self, ui: &mut Ui, id: Id, size: Vec2) -> Response;

    /// Shows the page in `response.rect`.
    ///
    /// An embedded view needs nothing here. A page that egui paints itself paints the texture
    /// here and reads egui's input.
    fn show(&self, ui: &mut Ui, response: &Response) {}
}

/// The [`PageWidget`] of a page in an embedded view (an `<iframe>` on the web).
///
/// It reserves the page's rect with [`egui::EmbeddedView`]. [`Backend::place`] then moves the
/// view there.
///
/// The view lives in the root window, so only show it in the root viewport.
#[derive(Clone, Copy, Debug, Default)]
pub struct EmbeddedWidget;

impl PageWidget for EmbeddedWidget {
    fn allocate(&self, ui: &mut Ui, id: Id, size: Vec2) -> Response {
        ui.add(egui::EmbeddedView::new(id).size(size))
    }
}

/// Trait implemented by all web view implementations.
///
/// A backend has two halves. The page itself lives in the backend. What egui shows of it is
/// the [`PageWidget`] from [`Self::widget`].
///
/// A backend either shows the page in an embedded view (an `<iframe>` on the web) that egui
/// doesn't draw and that [`Self::place`] positions, or renders the page into a texture that
/// its [`PageWidget`] paints.
///
/// All methods are called on the thread that runs egui, so a backend doesn't need to be `Send`.
#[expect(unused_variables)] // The default implementations ignore their arguments.
pub trait Backend {
    /// Load a page.
    ///
    /// # Errors
    /// If the page can't start loading, such as for a URL the engine can't parse. Errors that
    /// occur while the page loads are not reported.
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error>;

    /// Execute this `script` in the page.
    ///
    /// # Errors
    /// If the script can't be delivered to the page, such as on the web for a cross-origin
    /// page. Most engines run the script asynchronously, so exceptions it throws are not
    /// reported.
    fn evaluate_script(&mut self, script: &str) -> Result<(), Error>;

    /// Reloads the current page. See [`crate::WebViewInstance::reload`].
    fn reload(&mut self);

    /// Navigates back in the webview's history. See [`crate::WebViewInstance::back`].
    fn back(&mut self);

    /// Navigates forward in the webview's history. See [`crate::WebViewInstance::forward`].
    fn forward(&mut self);

    /// The half of this page that runs in egui's pass.
    ///
    /// A backend with an embedded view returns [`EmbeddedWidget`], and [`Self::place`] positions
    /// the view. A backend that renders into a texture returns a widget that paints the texture
    /// and passes egui's input on to the page.
    fn widget(&self) -> Arc<dyn PageWidget>;

    /// Called by [`crate::WebViewInstance::place`] after a pass of the root viewport.
    ///
    /// A backend with an embedded view positions it here. `output` is what egui reported for the
    /// webview's [`egui::EmbeddedView`]. It is `None` when the webview wasn't shown in the pass,
    /// and the backend should then hide the page.
    ///
    /// A backend that renders into a texture always gets `None`. It can use the call for work
    /// that has to happen after every pass.
    ///
    /// The rects of `output` are in ui points. Multiply them by [`Context::zoom_factor`] to get
    /// window points, or by [`Context::pixels_per_point`] to get physical pixels.
    fn place(&mut self, ctx: &Context, output: Option<&EmbeddedViewOutput>) {}

    /// Gives the page the keyboard focus, because egui's focus moved to the webview.
    ///
    /// `direction` says how egui's focus arrived:
    /// - [`FocusDirection::Next`] (Tab): focus the page's first focusable element.
    /// - [`FocusDirection::Previous`] (Shift-Tab): focus the page's last focusable element.
    /// - Any other direction, such as after [`egui::Response::request_focus`]: keep the element
    ///   the page last had focused.
    fn focus(&mut self, direction: FocusDirection) {}

    /// Returns the keyboard focus to egui, because egui's focus left the webview.
    ///
    /// Only move the focus if the page, or another view that belongs to this backend, has it.
    /// Another webview may already hold the focus.
    fn blur(&mut self) {}

    /// Whether the page has the keyboard focus.
    ///
    /// The webview polls this while it doesn't consider the page focused, to detect the page
    /// taking the focus by itself, such as when the user clicks it.
    ///
    /// A backend that forwards egui's key events to the page keeps the default `false`: its
    /// page is focused exactly when the webview has egui's focus.
    fn has_focus(&self) -> bool {
        false
    }

    /// Returns the direction in which the keyboard focus left the page by itself since the last
    /// call, if it did.
    ///
    /// [`FocusDirection::Next`] means Tab moved past the page's last element, and
    /// [`FocusDirection::Previous`] means Shift-Tab moved before its first. egui then moves its
    /// focus to the next or previous widget.
    fn poll_focus_left(&mut self) -> Option<FocusDirection> {
        None
    }

    /// Returns the events that occurred since the last call, oldest first.
    fn take_events(&mut self) -> Vec<WebViewEvent>;
}

/// Returns a script that focuses the first element in the page's sequential focus navigation
/// order for [`FocusDirection::Next`], or the last one for [`FocusDirection::Previous`].
///
/// Engines report when Tab leaves the page, but some can't move the focus to the page's first
/// or last element themselves; those run this script. Returns `None` for other directions.
///
/// The script approximates the browser's order: elements with a positive `tabindex` in
/// ascending order, then the other focusable elements in document order. It skips hidden and
/// disabled elements.
///
/// [`FocusDirection::Next`]: egui::FocusDirection::Next
/// [`FocusDirection::Previous`]: egui::FocusDirection::Previous
#[cfg(target_arch = "wasm32")]
pub fn focus_script(direction: egui::FocusDirection) -> Option<String> {
    let index = match direction {
        egui::FocusDirection::Next => 0,
        egui::FocusDirection::Previous => -1,
        _ => return None,
    };
    Some(format!(
        r#"(() => {{
    const candidates = document.querySelectorAll(
        "a[href], area[href], button, input, select, textarea, iframe, summary, " +
        "audio[controls], video[controls], [contenteditable]:not([contenteditable=false]), [tabindex]");
    const tabbable = [...candidates].filter((element) =>
        element.tabIndex >= 0 && !element.disabled &&
        !(element instanceof HTMLInputElement && element.type === "hidden") &&
        element.getClientRects().length > 0 &&
        getComputedStyle(element).visibility !== "hidden");
    const order = [
        ...tabbable.filter((element) => element.tabIndex > 0).sort((a, b) => a.tabIndex - b.tabIndex),
        ...tabbable.filter((element) => element.tabIndex === 0),
    ];
    order.at({index})?.focus();
}})();"#
    ))
}
