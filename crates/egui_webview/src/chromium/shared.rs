//! What the CEF handlers of one page, its widget and its backend share.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use egui::mutex::Mutex;
use egui::{Context, CursorIcon, FocusDirection, Rect, Vec2, ViewportId};

use super::handlers::ContextMenu;
use super::layer::Layer;
use crate::WebViewEvent;

/// The state that the CEF handlers share with the [`super::CefBackend`].
pub struct Shared {
    pub ctx: Context,
    /// What happened in the page, for [`crate::Backend::take_events`].
    pub events: Mutex<Vec<WebViewEvent>>,
    /// The size of the view in points, and the pixels per point.
    ///
    /// CEF's view coordinates (DIPs) are egui points, with the origin at the view's top-left.
    pub size: Mutex<(Vec2, f32)>,
    /// The latest frame of the page.
    pub view: Mutex<Layer>,
    /// The latest frame of the popup widget, such as the drop-down list of a `<select>`.
    pub popup: Mutex<Layer>,
    /// The popup widget's rect in view points, while it is open.
    pub popup_rect: Mutex<Option<Rect>>,
    pub cursor: Mutex<CursorIcon>,
    pub tooltip: Mutex<Option<String>>,
    /// Whether an editable element of the page has focus.
    pub editable: AtomicBool,
    /// The text cursor at the end of the IME composition, in view points, for placing the IME's
    /// candidate window.
    pub text_cursor: Mutex<Option<Rect>>,
    /// The context menu CEF asked to show, until egui closes it.
    pub context_menu: Mutex<Option<ContextMenu>>,
    /// Which way Tab left the page, until [`crate::Backend::poll_focus_left`] picks it up.
    pub took_focus_away: Mutex<Option<FocusDirection>>,

    /// What the widget wrote down in its passes, until the backend acts on it after the pass.
    pub inbox: Mutex<Inbox>,
    /// The root viewport's pass (see [`crate::mailbox::now`]) that last showed the page.
    pub shown_pass: Mutex<Option<u64>>,
}

/// What the page's widget saw and did in its passes, for the backend.
///
/// The widget can't reach the browser. So it writes this down during the pass, and the backend
/// tells the browser after the pass.
#[derive(Default)]
pub struct Inbox {
    /// The input of each pass that showed the page, oldest first.
    pub batches: Vec<super::input::Batch>,

    /// The context menu that egui closed, and the ID of the command the user picked, if any.
    pub menu_done: Option<(ContextMenu, Option<i32>)>,
}

impl Shared {
    pub fn new(ctx: &Context) -> Arc<Self> {
        Arc::new(Self {
            ctx: ctx.clone(),
            events: Mutex::default(),
            size: Mutex::new((egui::vec2(1.0, 1.0), ctx.pixels_per_point())),
            view: Mutex::default(),
            popup: Mutex::default(),
            popup_rect: Mutex::new(None),
            cursor: Mutex::new(CursorIcon::Default),
            tooltip: Mutex::new(None),
            editable: AtomicBool::new(false),
            text_cursor: Mutex::new(None),
            context_menu: Mutex::new(None),
            took_focus_away: Mutex::new(None),
            inbox: Mutex::default(),
            shown_pass: Mutex::new(None),
        })
    }

    /// Queues `event` for [`crate::Backend::take_events`], and requests a repaint so that the
    /// widget picks it up.
    pub fn send(&self, event: WebViewEvent) {
        self.events.lock().push(event);
        self.request_repaint();
    }

    /// Requests a repaint of the root viewport, which is the one that shows the page.
    ///
    /// CEF calls the handlers during a pass of whichever viewport runs the message loop work,
    /// which isn't necessarily the root.
    pub fn request_repaint(&self) {
        self.ctx.request_repaint_of(ViewportId::ROOT);
    }
}
