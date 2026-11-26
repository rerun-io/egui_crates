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
    client::EguiClient::new(
        Arc::clone(context),
        client::Handlers {
            render: render::EguiRenderHandler::new(Arc::clone(context)),
            display: display::EguiDisplayHandler::new(Arc::clone(context)),
            load: load::EguiLoadHandler::new(Arc::clone(context)),
            life_span: life_span::EguiLifeSpanHandler::new(Arc::clone(context)),
            context_menu: context_menu::EguiContextMenuHandler::new(Arc::clone(context)),
            focus: focus::EguiFocusHandler::new(Arc::clone(context)),
            request: request::EguiRequestHandler::new(Arc::clone(context)),
        },
    )
}
