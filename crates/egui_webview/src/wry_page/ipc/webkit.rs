//! A `WKScriptMessageHandler` that only accepts messages from the page's main frame.
//!
//! wry's own handler also accepts the messages of subframes.

#![expect(unsafe_code)] // WebKit

use objc2::rc::Retained;
use objc2::runtime::{NSObject, ProtocolObject};
use objc2::{DefinedClass as _, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSObjectProtocol, NSString, ns_string};
use objc2_web_kit::{WKScriptMessage, WKScriptMessageHandler, WKUserContentController};

use wry::WebViewExtMacOS as _;

use super::OnMessage;

/// Installed at document start in the main frame, including after navigation.
const SCRIPT: &str = r#"
Object.defineProperty(window, "ipc", {
    value: Object.freeze({ postMessage: text => window.webkit.messageHandlers.ipc.postMessage(text) })
});
"#;

struct HandlerIvars {
    on_message: OnMessage,
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
                    (self.ivars().on_message)(text.to_string());
                }
            }
        }
    }
);

/// Gives every document of the webview a `window.ipc` that posts to the handler of [`Ipc`].
pub fn prepare<'a>(
    builder: wry::WebViewBuilder<'a>,
    _on_message: &OnMessage,
) -> wry::WebViewBuilder<'a> {
    builder.with_initialization_script(SCRIPT)
}

/// Owns the handler registration for one webview.
pub struct Ipc(Retained<WKUserContentController>);

impl Ipc {
    /// Registers the handler. Call this before the first document can run its scripts.
    pub fn new(webview: &wry::WebView, on_message: OnMessage) -> Self {
        let controller = webview.manager();
        let handler = Handler::alloc(controller.mtm()).set_ivars(HandlerIvars { on_message });
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
