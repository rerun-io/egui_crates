//! Messages that the page's main frame sends with `window.ipc.postMessage(text)`.
//!
//! Each platform has one implementation, with the same two steps: [`prepare`] the builder of
//! the webview, then keep an [`Ipc`] for as long as the webview lives.

use std::rc::Rc;

cfg_select! {
    target_os = "macos" => {
        mod webkit;
        pub(super) use webkit::{Ipc, prepare};
    }
    _ => {
        mod builtin;
        pub(super) use builtin::{Ipc, prepare};
    }
}

/// Called with the text of each message from the main frame.
pub(super) type OnMessage = Rc<dyn Fn(String)>;
