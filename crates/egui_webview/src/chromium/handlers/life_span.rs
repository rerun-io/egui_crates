//! Links that would open a new window.

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, BrowserSettings, CefString, Client, ImplBrowser as _, ImplFrame as _,
    ImplLifeSpanHandler, LifeSpanHandler, WindowInfo, WrapLifeSpanHandler, wrap_life_span_handler,
};

wrap_life_span_handler! {
    pub struct EguiLifeSpanHandler {}

    impl LifeSpanHandler {
        /// Loads a link that would open a new window (such as `target="_blank"` or
        /// `window.open`) in this browser instead, and cancels the new window. A windowless
        /// browser has nowhere to show one.
        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: cef::WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&cef::PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<cef::DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let (Some(frame), Some(url)) =
                (browser.and_then(|b| b.main_frame()), target_url)
            {
                frame.load_url(Some(url));
            }
            1
        }
    }
}
