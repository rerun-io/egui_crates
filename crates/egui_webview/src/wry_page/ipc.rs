//! The native endpoint for messages from the page's main frame.

#![expect(unsafe_code)] // WebKit

use std::cell::RefCell;
use std::rc::Rc;

use egui::Context;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{DefinedClass as _, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSObjectProtocol, NSString, ns_string};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

use crate::WebViewEvent;

/// Installed at document start in the main frame, including after navigation.
pub(super) const SCRIPT: &str = r#"
Object.defineProperty(window, "ipc", {
    value: Object.freeze({ postMessage: text => window.webkit.messageHandlers.ipc.postMessage(text) })
});
"#;

struct HandlerIvars {
    ctx: Context,
    events: Rc<RefCell<Vec<WebViewEvent>>>,
}

define_class!(
    /// Checks the native sender before publishing a message to the host.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewMessageHandler"]
    #[ivars = HandlerIvars]
    struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl WKScriptMessageHandler for Handler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: WebKit supplies this message on the main thread. Its frame and body
            // are read-only, and the body is checked before it is used as a string.
            unsafe {
                if message.frameInfo().isMainFrame()
                    && let Ok(text) = message.body().downcast::<NSString>()
                {
                    let HandlerIvars { ctx, events } = self.ivars();
                    events
                        .borrow_mut()
                        .push(WebViewEvent::Message(text.to_string()));
                    ctx.request_repaint();
                }
            }
        }
    }
);

/// Owns the handler registration for one webview.
pub(super) struct Ipc(Retained<WKUserContentController>);

impl Ipc {
    pub(super) fn new(
        ctx: &Context,
        controller: Retained<WKUserContentController>,
        events: Rc<RefCell<Vec<WebViewEvent>>>,
    ) -> Self {
        let handler = Handler::alloc(controller.mtm()).set_ivars(HandlerIvars {
            ctx: ctx.clone(),
            events,
        });
        // SAFETY: NSObject supports init. This controller is private to the new webview,
        // and retains the handler until Drop unregisters it.
        unsafe {
            let handler: Retained<Handler> = msg_send![super(handler), init];
            controller.addScriptMessageHandler_name(
                ProtocolObject::from_ref(&*handler),
                ns_string!("ipc"),
            );
        }
        Self(controller)
    }
}

impl Drop for Ipc {
    fn drop(&mut self) {
        // SAFETY: This removes our registration from the same controller.
        unsafe {
            self.0.removeScriptMessageHandlerForName(ns_string!("ipc"));
        }
    }
}
