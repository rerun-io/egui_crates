//! The keyboard focus leaving the page.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{Browser, FocusHandler, ImplFocusHandler, WrapFocusHandler, wrap_focus_handler};
use egui::FocusDirection;

use crate::chromium::context::Context;

wrap_focus_handler! {
    pub struct EguiFocusHandler {
        context: Arc<Context>,
    }

    impl FocusHandler {
        /// Called when Tab moves the focus past the page's last element (`next` is nonzero), or
        /// Shift-Tab past its first.
        fn on_take_focus(&self, _browser: Option<&mut Browser>, next: ::std::os::raw::c_int) {
            self.context.state.lock().took_focus_away = Some(if next == 0 {
                FocusDirection::Previous
            } else {
                FocusDirection::Next
            });
            self.context.request_repaint();
        }
    }
}
