//! Links that would open a new window, reported as [`WebViewEvent::NewWindowRequested`].

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, BrowserSettings, CefString, Client, ImplLifeSpanHandler, LifeSpanHandler, WindowInfo,
    WrapLifeSpanHandler, wrap_life_span_handler,
};

use crate::chromium::context::Context;
use crate::{WebViewEvent, reported_url};

wrap_life_span_handler! {
    pub struct EguiLifeSpanHandler {
        context: Arc<Context>,
    }

    impl LifeSpanHandler {
        /// Cancels a new window (such as `target="_blank"` or `window.open`), and tells the app
        /// about it instead. A windowless browser has nowhere to show one, and loading it in
        /// this browser would let any script or `<iframe>` navigate the whole view.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: cef::WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&cef::PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<cef::DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let Some(url) = target_url.and_then(|url| reported_url(&url.to_string())) {
                self.context.send(WebViewEvent::NewWindowRequested {
                    url,
                    user_gesture: user_gesture != 0,
                });
            }
            let cancel = true;
            cancel.into()
        }
    }
}
