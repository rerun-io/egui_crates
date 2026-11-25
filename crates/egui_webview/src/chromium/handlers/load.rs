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
            let frame = frame.filter(|frame| frame.is_main() == 1);
            // CEF has no scripts that run at the start of a document. This is the first that
            // the browser process hears of the document, and the script usually runs before
            // the first paint.
            // The state must not stay locked across the call into CEF.
            let script = self.context.state.lock().document_start_script.clone();
            if let Some(frame) = &frame
                && let Some(script) = script
            {
                frame.execute_java_script(Some(&script.as_str().into()), None, 0);
            }
            if let Some(url) = main_frame_url(frame) {
                // The render process may report no focus change when the focused page goes away.
                self.context.state.lock().focused_node_editable = false;
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
