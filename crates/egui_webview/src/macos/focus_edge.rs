use std::cell::Cell;
use std::rc::Rc;

use egui::{Context, FocusDirection};
use objc2::rc::Retained;
use objc2::{DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSView;

pub(super) struct FocusEdgeIvars {
    /// The direction in which the focus left the page when this view becomes first responder.
    direction: FocusDirection,

    /// Shared with [`super::WryBackend::focus_left`]. Set when this view becomes first
    /// responder.
    focus_left: Rc<Cell<Option<FocusDirection>>>,

    /// Repainted when this view becomes first responder, so that the webview polls
    /// [`crate::Backend::poll_focus_left`].
    ctx: Context,
}

define_class!(
    /// An empty view just before or after the `WKWebView` in the window's key view loop.
    ///
    /// When Tab leaves the page's last element, `WebKit` makes the next key view first
    /// responder, and the previous one for Shift-Tab. A focus edge that becomes first responder
    /// thus reports that the focus left the page, and in which direction.
    ///
    /// Conversely, selecting the key view that follows the edge before the page makes `WebKit`
    /// focus the page's first element, and selecting the one that precedes the edge after the
    /// page focuses its last element.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewFocusEdge"]
    #[ivars = FocusEdgeIvars]
    pub(super) struct FocusEdge;

    impl FocusEdge {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// Joins the key view loop even when "Keyboard navigation" is off in System Settings.
        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ivars = self.ivars();
            ivars.focus_left.set(Some(ivars.direction));
            ivars.ctx.request_repaint();
            true
        }
    }
);

impl FocusEdge {
    pub(super) fn new(
        mtm: MainThreadMarker,
        ctx: &Context,
        direction: FocusDirection,
        focus_left: &Rc<Cell<Option<FocusDirection>>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FocusEdgeIvars {
            direction,
            focus_left: Rc::clone(focus_left),
            ctx: ctx.clone(),
        });
        // SAFETY: `NSView` supports `init`, which calls `initWithFrame:` with a zero rect.
        unsafe { msg_send![super(this), init] }
    }
}
