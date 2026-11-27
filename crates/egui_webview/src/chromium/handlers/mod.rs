//! CEF's client handlers for one browser: off-screen rendering, cursor and tooltip, focus, page
//! loads, new windows, the context menu, the end of the render process, and IPC messages from
//! the page.

mod client;
mod context_menu;
mod display;
mod focus;
mod life_span;
mod load;
mod render;
mod request;

use std::sync::Arc;

use crate::chromium::context::Context;

pub use context_menu::{ContextMenu, MenuItem};

/// Creates the CEF client of a browser that renders into `context`.
pub fn client(context: &Arc<Context>) -> cef::Client {
    client::EguiClient::new(Arc::clone(context), client::Handlers::new(context))
}
