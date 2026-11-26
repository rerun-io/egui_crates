//! The end of the page's render process, reported as [`WebViewEvent::Crashed`].

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, ImplRequestHandler, RequestHandler, TerminationStatus, WrapRequestHandler,
    wrap_request_handler,
};

use crate::WebViewEvent;
use crate::chromium::context::Context;

wrap_request_handler! {
    pub struct EguiRequestHandler {
        context: Arc<Context>,
    }

    impl RequestHandler {
        /// The render process is gone, and with it the page. The browser stays blank until it
        /// loads a page again.
        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: ::std::os::raw::c_int,
            error_string: Option<&CefString>,
        ) {
            let status = status.as_ref();
            let detail = error_string.map(ToString::to_string).unwrap_or_default();
            let reason = if detail.is_empty() {
                format!("{status:?} (code {error_code})")
            } else {
                format!("{status:?}: {detail} (code {error_code})")
            };
            log::warn!("egui_webview: the render process of a CEF page ended: {reason}");
            self.context.send(WebViewEvent::Crashed { reason });
        }
    }
}
