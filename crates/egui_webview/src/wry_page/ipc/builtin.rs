//! wry's own message handler.
//!
//! Only for platforms where wry reports the messages of the main frame alone.

use std::rc::Rc;

use super::OnMessage;

/// Makes the webview report its messages to `on_message`.
pub fn prepare<'a>(
    builder: wry::WebViewBuilder<'a>,
    on_message: &OnMessage,
) -> wry::WebViewBuilder<'a> {
    let on_message = Rc::clone(on_message);
    builder.with_ipc_handler(move |request| on_message(request.into_body()))
}

/// Nothing to own: wry's handler lives in the webview.
pub struct Ipc;

impl Ipc {
    pub fn new(_webview: &wry::WebView, _on_message: OnMessage) -> Self {
        Self
    }
}
