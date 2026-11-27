//! Page loads, reported as [`WebViewEvent`]s.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, ImplFrame as _, ImplLoadHandler, LoadHandler, WrapLoadHandler,
    wrap_load_handler,
};

use crate::chromium::context::Context;
use crate::{Url, WebViewEvent, reported_url};

wrap_load_handler! {
    pub struct EguiLoadHandler {
        context: Arc<Context>,
    }

    impl LoadHandler {
        fn on_load_start(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: cef::TransitionType,
        ) {
            if let Some(url) = main_frame_url(frame) {
                {
                    let mut state = self.context.state.lock();
                    // The render process may report no focus change when the focused page goes
                    // away.
                    state.focused_node_editable = false;
                    state.load_failed = false;
                }
                self.context.send(WebViewEvent::Loading { url });
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            // CEF also ends a load that failed, which `on_load_error` has reported.
            if let Some(url) = main_frame_url(frame)
                && !self.context.state.lock().load_failed
            {
                self.context.send(WebViewEvent::Loaded { url });
            }
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            error_code: cef::Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let is_main_frame = frame.is_some_and(|frame| frame.is_main() == 1);
            // A load is aborted when another one replaces it, or when it becomes a download.
            if !is_main_frame || error_code == cef::Errorcode::ABORTED {
                return;
            }
            let Some(url) = failed_url.and_then(|url| reported_url(&url.to_string())) else {
                return;
            };
            let code = error_code.as_ref();
            let text = error_text.map(ToString::to_string).unwrap_or_default();
            let error = if text.is_empty() {
                format!("{code:?}")
            } else {
                format!("{text} ({code:?})")
            };
            self.context.state.lock().load_failed = true;
            self.context.send(WebViewEvent::LoadFailed { url, error });
        }
    }
}

/// The URL of `frame`, if it is the main frame and shows a page.
///
/// Chromium's own error page, which replaces a page that failed to load, doesn't count.
fn main_frame_url(frame: Option<&mut cef::Frame>) -> Option<Url> {
    let frame = frame.filter(|frame| frame.is_main() == 1)?;
    reported_url(&CefString::from(&frame.url()).to_string())
        .filter(|url| url.scheme() != "chrome-error")
}
