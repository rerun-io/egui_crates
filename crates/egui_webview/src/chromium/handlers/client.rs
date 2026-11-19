//! The client that hands CEF the other handlers, and receives the page's IPC messages.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, Client, ContextMenuHandler, DisplayHandler, FocusHandler, ImplClient,
    ImplFrame as _, ImplListValue as _, ImplProcessMessage as _, LifeSpanHandler, LoadHandler,
    RenderHandler, WrapClient, wrap_client,
};

use crate::WebViewEvent;
use crate::chromium::runtime::IPC_MESSAGE;
use crate::chromium::shared::Shared;

wrap_client! {
    pub struct EguiClient {
        shared: Arc<Shared>,
        render: RenderHandler,
        display: DisplayHandler,
        load: LoadHandler,
        life_span: LifeSpanHandler,
        context_menu: ContextMenuHandler,
        focus: FocusHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display.clone())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(self.load.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life_span.clone())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(self.context_menu.clone())
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            Some(self.focus.clone())
        }

        /// Turns the text that the main frame passed to `window.ipc.postMessage(text)` into a
        /// [`WebViewEvent::Message`].
        fn on_process_message_received(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _source_process: cef::ProcessId,
            message: Option<&mut cef::ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            // Only the main frame defines `window.ipc`, but the sender is checked here too,
            // since a compromised render process can send any message.
            let Some(message) = message.filter(|_| frame.is_some_and(|frame| frame.is_main() != 0))
            else {
                return 0;
            };
            if CefString::from(&message.name()).to_string() != IPC_MESSAGE {
                return 0;
            }
            let text = message
                .argument_list()
                .map(|arguments| CefString::from(&arguments.string(0)).to_string())
                .unwrap_or_default();
            self.shared.send(WebViewEvent::Message(text));
            1
        }
    }
}
