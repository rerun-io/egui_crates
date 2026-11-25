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
                self.context.send(WebViewEvent::Loading { url });
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            if let Some(url) = main_frame_url(frame) {
                self.context.send(WebViewEvent::Loaded { url });
            }
        }
    }
}

/// The URL of `frame`, if it is the main frame.
fn main_frame_url(frame: Option<&mut cef::Frame>) -> Option<Url> {
    let frame = frame.filter(|frame| frame.is_main() == 1)?;
    reported_url(&CefString::from(&frame.url()).to_string())
}
