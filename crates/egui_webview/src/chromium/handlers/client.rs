//! The client that hands CEF the other handlers, and receives the page's IPC messages.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, Client, ContextMenuHandler, DisplayHandler, FocusHandler, ImplClient,
    ImplFrame as _, ImplListValue as _, ImplProcessMessage as _, LifeSpanHandler, LoadHandler,
    RenderHandler, RequestHandler, WrapClient, wrap_client,
};

use super::{context_menu, display, focus, life_span, load, render, request};
use crate::WebViewEvent;
use crate::chromium::context::Context;
use crate::chromium::runtime::{FOCUSED_NODE_MESSAGE, IPC_MESSAGE};

/// The handlers that [`EguiClient`] hands to CEF.
#[derive(Clone)]
pub struct Handlers {
    pub render: RenderHandler,
    pub display: DisplayHandler,
    pub load: LoadHandler,
    pub life_span: LifeSpanHandler,
    pub context_menu: ContextMenuHandler,
    pub focus: FocusHandler,
    pub request: RequestHandler,
}

impl Handlers {
    /// Creates the handlers of a browser that renders into `context`.
    pub fn new(context: &Arc<Context>) -> Self {
        Self {
            render: render::EguiRenderHandler::new(Arc::clone(context)),
            display: display::EguiDisplayHandler::new(Arc::clone(context)),
            load: load::EguiLoadHandler::new(Arc::clone(context)),
            life_span: life_span::EguiLifeSpanHandler::new(Arc::clone(context)),
            context_menu: context_menu::EguiContextMenuHandler::new(Arc::clone(context)),
            focus: focus::EguiFocusHandler::new(Arc::clone(context)),
            request: request::EguiRequestHandler::new(Arc::clone(context)),
        }
    }
}

wrap_client! {
    pub struct EguiClient {
        context: Arc<Context>,
        handlers: Handlers,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.handlers.render.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.handlers.display.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.handlers.load.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.handlers.life_span.clone())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.handlers.context_menu.clone())
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(self.handlers.focus.clone())
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(self.handlers.request.clone())
        }

        /// Turns the text that the main frame passed to `window.ipc.postMessage(text)` into a
        /// [`WebViewEvent::Message`], and stores whether a frame's newly focused element is
        /// editable.
        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _source_process: cef::ProcessId,
            message: Option<&mut cef::ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            let Some(message) = message else {
                return 0;
            };
            match CefString::from(&message.name()).to_string().as_str() {
                // Every frame reports its focus changes, and the message carries only a bool.
                FOCUSED_NODE_MESSAGE => {
                    let editable = message
                        .argument_list()
                        .is_some_and(|arguments| arguments.bool(0) != 0);
                    self.context.state.lock().focused_node_editable = editable;
                    self.context.request_repaint();
                    1
                }
                // Only the main frame defines `window.ipc`, but the sender is checked here too,
                // since a compromised render process can send any message.
                IPC_MESSAGE if frame.is_some_and(|frame| frame.is_main() != 0) => {
                    let text = message
                        .argument_list()
                        .map(|arguments| CefString::from(&arguments.string(0)).to_string())
                        .unwrap_or_default();
                    self.context.send(WebViewEvent::Message(text));
                    1
                }
                _ => 0,
            }
        }
    }
}
