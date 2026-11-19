//! CEF's client handlers for one browser: off-screen rendering, cursor and tooltip, focus, page
//! loads, new windows, the context menu, and IPC messages from the page.

mod client;
mod context_menu;
mod display;
mod focus;
mod life_span;
mod load;
mod render;

use std::sync::Arc;

use crate::chromium::shared::Shared;

pub use context_menu::{ContextMenu, MenuItem};

/// Creates the CEF client of a browser that renders into `shared`.
pub fn client(shared: &Arc<Shared>) -> cef::Client {
    client::EguiClient::new(
        Arc::clone(shared),
        render::EguiRenderHandler::new(Arc::clone(shared)),
        display::EguiDisplayHandler::new(Arc::clone(shared)),
        load::EguiLoadHandler::new(Arc::clone(shared)),
        life_span::EguiLifeSpanHandler::new(),
        context_menu::EguiContextMenuHandler::new(Arc::clone(shared)),
        focus::EguiFocusHandler::new(Arc::clone(shared)),
    )
}
