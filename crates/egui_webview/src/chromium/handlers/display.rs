//! The page's cursor and tooltip.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, DisplayHandler, ImplDisplayHandler, WrapDisplayHandler, sys,
    wrap_display_handler,
};
use egui::CursorIcon;

use crate::chromium::context::Context;

/// The native cursor handle CEF passes to `on_cursor_change`, whose type differs per platform.
#[cfg(not(target_os = "macos"))]
use cef::CursorHandle;
#[cfg(target_os = "macos")]
type CursorHandle = *mut u8;

wrap_display_handler! {
    pub struct EguiDisplayHandler {
        context: Arc<Context>,
    }

    impl DisplayHandler {
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: CursorHandle,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&cef::CursorInfo>,
        ) -> ::std::os::raw::c_int {
            self.context.state.lock().cursor = cursor_icon(*type_.as_ref());
            self.context.request_repaint();
            1
        }

        fn on_tooltip(
            &self,
            _browser: Option<&mut Browser>,
            text: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let text = text.map(|t| t.to_string()).filter(|t| !t.is_empty());
            self.context.state.lock().tooltip = text;
            1
        }
    }
}

fn cursor_icon(cursor: sys::cef_cursor_type_t) -> CursorIcon {
    use sys::cef_cursor_type_t as C;
    match cursor {
        C::CT_CROSS => CursorIcon::Crosshair,
        C::CT_HAND => CursorIcon::PointingHand,
        C::CT_IBEAM => CursorIcon::Text,
        C::CT_VERTICALTEXT => CursorIcon::VerticalText,
        C::CT_WAIT => CursorIcon::Wait,
        C::CT_PROGRESS => CursorIcon::Progress,
        C::CT_HELP => CursorIcon::Help,
        C::CT_EASTRESIZE => CursorIcon::ResizeEast,
        C::CT_NORTHRESIZE => CursorIcon::ResizeNorth,
        C::CT_NORTHEASTRESIZE => CursorIcon::ResizeNorthEast,
        C::CT_NORTHWESTRESIZE => CursorIcon::ResizeNorthWest,
        C::CT_SOUTHRESIZE => CursorIcon::ResizeSouth,
        C::CT_SOUTHEASTRESIZE => CursorIcon::ResizeSouthEast,
        C::CT_SOUTHWESTRESIZE => CursorIcon::ResizeSouthWest,
        C::CT_WESTRESIZE => CursorIcon::ResizeWest,
        C::CT_NORTHSOUTHRESIZE | C::CT_ROWRESIZE => CursorIcon::ResizeVertical,
        C::CT_EASTWESTRESIZE | C::CT_COLUMNRESIZE => CursorIcon::ResizeHorizontal,
        C::CT_NORTHEASTSOUTHWESTRESIZE => CursorIcon::ResizeNeSw,
        C::CT_NORTHWESTSOUTHEASTRESIZE => CursorIcon::ResizeNwSe,
        C::CT_MOVE | C::CT_MIDDLEPANNING => CursorIcon::Move,
        C::CT_CELL => CursorIcon::Cell,
        C::CT_CONTEXTMENU => CursorIcon::ContextMenu,
        C::CT_ALIAS | C::CT_DND_LINK => CursorIcon::Alias,
        C::CT_COPY | C::CT_DND_COPY => CursorIcon::Copy,
        C::CT_NODROP | C::CT_DND_NONE => CursorIcon::NoDrop,
        C::CT_NOTALLOWED => CursorIcon::NotAllowed,
        C::CT_ZOOMIN => CursorIcon::ZoomIn,
        C::CT_ZOOMOUT => CursorIcon::ZoomOut,
        C::CT_GRAB => CursorIcon::Grab,
        C::CT_GRABBING | C::CT_DND_MOVE => CursorIcon::Grabbing,
        C::CT_NONE => CursorIcon::None,
        _ => CursorIcon::Default,
    }
}
