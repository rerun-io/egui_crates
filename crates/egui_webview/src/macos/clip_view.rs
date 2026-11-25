use std::cell::RefCell;

use egui::{EmbeddedViewOutput, Rect};
use objc2::rc::Retained;
use objc2::{DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::NSView;
use objc2_core_foundation::CGPoint;

#[derive(Default)]
pub(super) struct ClipIvars {
    /// The areas where egui takes the pointer, in the clip view's own (flipped) coordinates.
    ///
    /// The clip view's hit-test returns `nil` inside them.
    pub(super) egui_input_areas: RefCell<Vec<Rect>>,
}

define_class!(
    /// A view that contains a `WKWebView` and clips it to [`EmbeddedViewOutput::visible`].
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewClipView"]
    #[ivars = ClipIvars]
    pub(super) struct ClipView;

    impl ClipView {
        /// Uses a top-left origin, like winit's view and egui.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Returns `nil` for a point in egui's input areas, so `AppKit` delivers the click to
        /// winit's view, and thus to egui.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: CGPoint) -> Option<Retained<NSView>> {
            // `hitTest:` gets `point` in the superview's coordinates.
            // SAFETY: `superview` returns an `NSView`, or nil.
            let superview = unsafe { self.superview() };
            let local = self.convertPoint_fromView(point, superview.as_deref());
            #[expect(clippy::cast_possible_truncation)]
            let local = egui::pos2(local.x as f32, local.y as f32);
            if self.ivars().egui_input_areas.borrow().iter().any(|area| area.contains(local)) {
                None
            } else {
                // SAFETY: The superclass method has the same signature.
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    }
);

impl ClipView {
    pub(super) fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClipIvars::default());
        // SAFETY: `NSView` supports `init`, which calls `initWithFrame:` with a zero rect.
        unsafe { msg_send![super(this), init] }
    }
}
