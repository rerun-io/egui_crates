//! What the CEF handlers of one page, its widget and its backend share.

use std::sync::Arc;
use std::sync::mpsc;

use egui::mutex::Mutex;
use egui::{CursorIcon, FocusDirection, Rect, Vec2, ViewportId};

use super::handlers::ContextMenu;
use super::layer::Layer;
use crate::WebViewEvent;

/// What the CEF handlers of one page share with its widget and its [`super::CefBackend`].
///
/// It is also the page's [`crate::PageWidget`], the half that runs in egui's pass.
pub struct Context {
    pub egui_ctx: egui::Context,

    /// Carries what happened in the page to [`crate::Backend::take_events`].
    events: mpsc::Sender<WebViewEvent>,

    /// Never keep this locked across a call into CEF. CEF can run a handler before it returns,
    /// and the handler locks the state too.
    pub state: Mutex<State>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("egui_ctx", &self.egui_ctx)
            .field("events", &self.events)
            // Leaves out `state`, because locking it here could deadlock with a CEF handler.
            .finish_non_exhaustive()
    }
}

/// Everything about a page that changes while it lives.
#[derive(Debug)]
pub struct State {
    /// The size of the view in points, and the pixels per point.
    ///
    /// CEF's view coordinates (DIPs) are egui points, with the origin at the view's top-left.
    pub size: (Vec2, f32),
    /// The latest frame of the page.
    pub view: Layer,
    /// The latest frame of the popup widget, such as the drop-down list of a `<select>`.
    pub popup: Layer,
    /// The popup widget's rect in view points, while it is open.
    pub popup_rect: Option<Rect>,
    pub cursor: CursorIcon,
    pub tooltip: Option<String>,
    /// Whether an editable element of the page has focus.
    pub editable: bool,
    /// The text cursor at the end of the IME composition, in view points, for placing the IME's
    /// candidate window.
    pub text_cursor: Option<Rect>,
    /// The context menu CEF asked to show, until egui closes it.
    pub context_menu: Option<ContextMenu>,
    /// Which way Tab left the page, until [`crate::Backend::poll_focus_left`] picks it up.
    pub took_focus_away: Option<FocusDirection>,

    /// What the widget wrote down in its passes, until the backend acts on it after the pass.
    pub inbox: Inbox,
    /// The root viewport's pass (see [`egui::Context::cumulative_pass_nr_for`]) that last showed
    /// the page.
    pub shown_pass: Option<u64>,
}

/// What the page's widget saw and did in its passes, for the backend.
///
/// The widget can't reach the browser. So it writes this down during the pass, and the backend
/// tells the browser after the pass.
#[derive(Debug, Default)]
pub struct Inbox {
    /// The input of each pass that showed the page, oldest first.
    pub batches: Vec<super::input::Batch>,

    /// The context menu that egui closed, and the ID of the command the user picked, if any.
    pub menu_done: Option<(ContextMenu, Option<i32>)>,
}

impl Context {
    /// Returns the context of a new page, and the receiving end of the page's events.
    pub fn new(egui_ctx: &egui::Context) -> (Arc<Self>, mpsc::Receiver<WebViewEvent>) {
        let (events, receiver) = mpsc::channel();
        let state = State {
            size: (egui::vec2(1.0, 1.0), egui_ctx.pixels_per_point()),
            view: Layer::default(),
            popup: Layer::default(),
            popup_rect: None,
            cursor: CursorIcon::Default,
            tooltip: None,
            editable: false,
            text_cursor: None,
            context_menu: None,
            took_focus_away: None,
            inbox: Inbox::default(),
            shown_pass: None,
        };
        let context = Arc::new(Self {
            egui_ctx: egui_ctx.clone(),
            events,
            state: Mutex::new(state),
        });
        super::runtime::register_page(&context);
        (context, receiver)
    }

    /// Queues `event` for [`crate::Backend::take_events`], and requests a repaint so that the
    /// widget picks it up.
    pub fn send(&self, event: WebViewEvent) {
        // CEF closes a browser some time after its backend is dropped. Nobody receives the
        // events of a page in between.
        self.events.send(event).ok();
        self.request_repaint();
    }

    /// Requests a repaint of the root viewport, which is the one that shows the page.
    ///
    /// CEF calls the handlers during a pass of whichever viewport runs the message loop work,
    /// which isn't necessarily the root.
    pub fn request_repaint(&self) {
        self.egui_ctx.request_repaint_of(ViewportId::ROOT);
    }
}
