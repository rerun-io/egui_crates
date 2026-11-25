//! Wakes egui when `AppKit` gives a page the keyboard focus.

use std::ffi::c_void;
use std::ptr;

use egui::Context;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{DefinedClass as _, MainThreadOnly, Message as _, define_class, msg_send};
use objc2_app_kit::{NSView, NSWindow};
use objc2_foundation::{
    NSDictionary, NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration as _,
    NSString, ns_string,
};

#[derive(Debug)]
pub(super) struct FocusObserverIvars {
    ctx: Context,
    view: Retained<NSView>,
    window: Retained<NSWindow>,
}

define_class!(
    /// Observes the window's first responder while the page exists.
    #[derive(Debug)]
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewFocusObserver"]
    #[ivars = FocusObserverIvars]
    pub(super) struct FocusObserver;

    impl FocusObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn first_responder_changed(
            &self,
            _key: &NSString,
            _object: &NSWindow,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            let FocusObserverIvars { ctx, view, window } = self.ivars();
            if window.firstResponder()
                .and_then(|responder| responder.downcast::<NSView>().ok())
                .is_some_and(|responder| responder.isDescendantOf(view))
            {
                ctx.request_repaint();
            }
        }
    }
);

impl FocusObserver {
    pub(super) fn new(ctx: &Context, window: Retained<NSWindow>, view: &NSView) -> Retained<Self> {
        let this = Self::alloc(window.mtm()).set_ivars(FocusObserverIvars {
            ctx: ctx.clone(),
            view: view.retain(),
            window,
        });
        // SAFETY: NSObject supports init. The only registration on this observer is for
        // NSWindow's KVO-compliant firstResponder property, and Drop removes it.
        unsafe {
            let this: Retained<Self> = msg_send![super(this), init];
            this.ivars().window.addObserver_forKeyPath_options_context(
                &this,
                ns_string!("firstResponder"),
                NSKeyValueObservingOptions::empty(),
                ptr::null_mut(),
            );
            this
        }
    }
}

impl Drop for FocusObserver {
    fn drop(&mut self) {
        // SAFETY: We still hold the window on which new registered this observer.
        unsafe {
            self.ivars()
                .window
                .removeObserver_forKeyPath(self, ns_string!("firstResponder"));
        }
    }
}
