//! CEF's client handlers for one browser: off-screen rendering, cursor and tooltip, focus, page
//! loads, new windows, the context menu, and IPC messages from the page.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::mutex::Mutex;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, BrowserSettings, CefString, Client, ContextMenuHandler, DisplayHandler, FocusHandler,
    ImplBrowser as _, ImplClient, ImplContextMenuHandler, ImplDisplayHandler, ImplFocusHandler,
    ImplFrame as _, ImplLifeSpanHandler, ImplLoadHandler, ImplMenuModel as _, ImplRenderHandler,
    LifeSpanHandler, LoadHandler, RenderHandler, WindowInfo, WrapClient, WrapContextMenuHandler,
    WrapDisplayHandler, WrapFocusHandler, WrapLifeSpanHandler, WrapLoadHandler, WrapRenderHandler,
    sys, wrap_client, wrap_context_menu_handler, wrap_display_handler, wrap_focus_handler,
    wrap_life_span_handler, wrap_load_handler, wrap_render_handler,
};
use egui::{
    Color32, ColorImage, Context, CursorIcon, FocusDirection, Pos2, Rect, TextureHandle,
    TextureOptions, Vec2, ViewportId, pos2,
};

use cef::{
    ImplContextMenuParams as _, ImplListValue as _, ImplProcessMessage as _,
    ImplRunContextMenuCallback as _,
};

use super::runtime::IPC_MESSAGE;
use crate::WebViewEvent;

/// The native cursor handle CEF passes to `on_cursor_change`, whose type differs per platform.
#[cfg(not(target_os = "macos"))]
use cef::CursorHandle;
#[cfg(target_os = "macos")]
type CursorHandle = *mut u8;

/// The state that the CEF handlers share with the [`super::CefBackend`].
pub struct Shared {
    pub ctx: Context,
    /// What happened in the page, for [`crate::Backend::take_events`].
    pub events: Mutex<Vec<WebViewEvent>>,
    /// The size of the view in points, and the pixels per point.
    ///
    /// CEF's view coordinates (DIPs) are egui points, with the origin at the view's top-left.
    pub size: Mutex<(Vec2, f32)>,
    /// The latest frame of the page.
    pub view: Mutex<Layer>,
    /// The latest frame of the popup widget, such as the drop-down list of a `<select>`.
    pub popup: Mutex<Layer>,
    /// The popup widget's rect in view points, while it is open.
    pub popup_rect: Mutex<Option<Rect>>,
    pub cursor: Mutex<CursorIcon>,
    pub tooltip: Mutex<Option<String>>,
    /// Whether an editable element of the page has focus.
    pub editable: AtomicBool,
    /// The text cursor at the end of the IME composition, in view points, for placing the IME's
    /// candidate window.
    pub text_cursor: Mutex<Option<Rect>>,
    /// The context menu CEF asked to show, until egui closes it.
    pub context_menu: Mutex<Option<ContextMenu>>,
    /// Which way Tab left the page, until [`crate::Backend::poll_focus_left`] picks it up.
    pub took_focus_away: Mutex<Option<FocusDirection>>,

    /// What the widget wrote down in its passes, until the backend acts on it after the pass.
    pub inbox: Mutex<Inbox>,
    /// The root viewport's pass (see [`crate::mailbox::now`]) that last showed the page.
    pub shown_pass: Mutex<Option<u64>>,
}

/// What the page's widget saw and did in its passes, for the backend.
///
/// The widget can't reach the browser. So it writes this down during the pass, and the backend
/// tells the browser after the pass.
#[derive(Default)]
pub struct Inbox {
    /// The input of each pass that showed the page, oldest first.
    pub batches: Vec<super::input::Batch>,

    /// The context menu that egui closed, and the ID of the command the user picked, if any.
    pub menu_done: Option<(ContextMenu, Option<i32>)>,
}

impl Shared {
    pub fn new(ctx: &Context) -> Arc<Self> {
        Arc::new(Self {
            ctx: ctx.clone(),
            events: Mutex::default(),
            size: Mutex::new((egui::vec2(1.0, 1.0), ctx.pixels_per_point())),
            view: Mutex::default(),
            popup: Mutex::default(),
            popup_rect: Mutex::new(None),
            cursor: Mutex::new(CursorIcon::Default),
            tooltip: Mutex::new(None),
            editable: AtomicBool::new(false),
            text_cursor: Mutex::new(None),
            context_menu: Mutex::new(None),
            took_focus_away: Mutex::new(None),
            inbox: Mutex::default(),
            shown_pass: Mutex::new(None),
        })
    }

    /// Queues `event` for [`crate::Backend::take_events`], and requests a repaint so that the
    /// widget picks it up.
    pub fn send(&self, event: WebViewEvent) {
        self.events.lock().push(event);
        self.request_repaint();
    }

    /// Requests a repaint of the root viewport, which is the one that shows the page.
    ///
    /// CEF calls the handlers during a pass of whichever viewport runs the message loop work,
    /// which isn't necessarily the root.
    pub fn request_repaint(&self) {
        self.ctx.request_repaint_of(ViewportId::ROOT);
    }

    /// Creates the CEF client of a browser that renders into this.
    pub fn client(self: &Arc<Self>) -> Client {
        let shared = Arc::clone(self);
        EguiClient::new(
            Arc::clone(&shared),
            EguiRenderHandler::new(Arc::clone(&shared)),
            EguiDisplayHandler::new(Arc::clone(&shared)),
            EguiLoadHandler::new(Arc::clone(&shared)),
            EguiLifeSpanHandler::new(),
            EguiContextMenuHandler::new(Arc::clone(&shared)),
            EguiFocusHandler::new(Arc::clone(&shared)),
        )
    }
}

/// The latest frame of the view or of its popup widget, as BGRA pixels with premultiplied alpha.
///
/// Sizes and rects are in physical pixels.
#[derive(Default)]
pub struct Layer {
    size: [usize; 2],
    bgra: Vec<u8>,
    /// The bounding rect of what changed since the last upload, as `[x, y, width, height]`.
    dirty: Option<[usize; 4]>,
    /// Whether the size changed since the last upload, so that the texture must be recreated.
    resized: bool,
    /// The egui texture that the frame is uploaded to.
    texture: Option<TextureHandle>,
}

impl Layer {
    /// Copies the `dirty` rects of a new frame from `buffer`. A frame of a new size replaces the
    /// whole layer.
    fn paint(&mut self, buffer: &[u8], size: [usize; 2], dirty: &[cef::Rect]) {
        if size != self.size {
            self.size = size;
            // Keep the allocation. A resize by dragging sends a new size in every frame.
            self.bgra.clear();
            self.bgra.extend_from_slice(buffer);
            self.resized = true;
            self.dirty = Some([0, 0, size[0], size[1]]);
            return;
        }
        let stride = size[0] * 4;
        for rect in dirty {
            let [x, y, w, h] = clamp_rect(rect, size);
            for row in y..y + h {
                let range = row * stride + x * 4..row * stride + (x + w) * 4;
                self.bgra[range.clone()].copy_from_slice(&buffer[range]);
            }
            self.dirty = Some(match self.dirty {
                None => [x, y, w, h],
                Some([dx, dy, dw, dh]) => {
                    let (x0, y0) = (dx.min(x), dy.min(y));
                    let (x1, y1) = ((dx + dw).max(x + w), (dy + dh).max(y + h));
                    [x0, y0, x1 - x0, y1 - y0]
                }
            });
        }
    }

    /// Uploads what changed to the texture, and returns it. Creates the texture if it doesn't
    /// exist yet or the size changed. Returns `None` until there is a first frame.
    pub fn upload(&mut self, ctx: &Context, name: &str) -> Option<&TextureHandle> {
        let Some([x, y, w, h]) = self.dirty.take() else {
            return self.texture.as_ref();
        };
        let stride = self.size[0] * 4;
        let image = |x: usize, y: usize, w: usize, h: usize| {
            let mut pixels = Vec::with_capacity(w * h);
            for row in y..y + h {
                let start = row * stride + x * 4;
                pixels.extend(
                    self.bgra[start..start + w * 4]
                        .chunks_exact(4)
                        .map(|p| Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3])),
                );
            }
            ColorImage::new([w, h], pixels)
        };
        match &mut self.texture {
            Some(texture) if !std::mem::take(&mut self.resized) => {
                texture.set_partial([x, y], image(x, y, w, h), TextureOptions::NEAREST);
            }
            texture => {
                *texture = Some(ctx.load_texture(
                    name,
                    image(0, 0, self.size[0], self.size[1]),
                    TextureOptions::NEAREST,
                ));
            }
        }
        self.texture.as_ref()
    }
}

/// `rect` as `[x, y, width, height]`, clipped to a frame of `size`.
fn clamp_rect(rect: &cef::Rect, size: [usize; 2]) -> [usize; 4] {
    let x = usize::try_from(rect.x).unwrap_or(0).min(size[0]);
    let y = usize::try_from(rect.y).unwrap_or(0).min(size[1]);
    let w = usize::try_from(rect.width).unwrap_or(0).min(size[0] - x);
    let h = usize::try_from(rect.height).unwrap_or(0).min(size[1] - y);
    [x, y, w, h]
}

wrap_render_handler! {
    struct EguiRenderHandler {
        shared: Arc<Shared>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut cef::Rect>) {
            if let Some(rect) = rect {
                let (size, _) = *self.shared.size.lock();
                // CEF requires a view rect that isn't empty.
                rect.x = 0;
                rect.y = 0;
                rect.width = super::round(size.x).max(1);
                rect.height = super::round(size.y).max(1);
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut cef::ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            let Some(screen_info) = screen_info else {
                return 0;
            };
            // CEF renders this many physical pixels per view point.
            screen_info.device_scale_factor = self.shared.size.lock().1;
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            if show == 0 {
                *self.shared.popup_rect.lock() = None;
                *self.shared.popup.lock() = Layer::default();
                self.shared.request_repaint();
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&cef::Rect>) {
            let Some(rect) = rect else {
                return;
            };
            // Move the popup inside the view, like cefclient does.
            let (size, _) = *self.shared.size.lock();
            let popup_size = egui::vec2(rect.width as f32, rect.height as f32);
            let min = pos2(rect.x as f32, rect.y as f32)
                .min((size - popup_size).to_pos2())
                .max(Pos2::ZERO);
            *self.shared.popup_rect.lock() =
                Some(Rect::from_min_size(min, popup_size));
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: cef::PaintElementType,
            dirty_rects: Option<&[cef::Rect]>,
            buffer: *const u8,
            width: ::std::os::raw::c_int,
            height: ::std::os::raw::c_int,
        ) {
            let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else {
                return;
            };
            if buffer.is_null() || width == 0 || height == 0 {
                return;
            }
            // SAFETY: CEF passes a buffer of `width * height` BGRA pixels of 4 bytes each, which
            // is valid for the duration of this call.
            #[expect(unsafe_code)]
            let buffer = unsafe { std::slice::from_raw_parts(buffer, width * height * 4) };
            let layer = if *type_.as_ref() == sys::cef_paint_element_type_t::PET_POPUP {
                &self.shared.popup
            } else {
                &self.shared.view
            };
            layer
                .lock()
                .paint(buffer, [width, height], dirty_rects.unwrap_or_default());
            self.shared.request_repaint();
        }

        fn on_virtual_keyboard_requested(
            &self,
            _browser: Option<&mut Browser>,
            input_mode: cef::TextInputMode,
        ) {
            // CEF asks for a virtual keyboard when an editable element gains focus, and passes
            // `CEF_TEXT_INPUT_MODE_NONE` when it loses it.
            let editable = *input_mode.as_ref() != sys::cef_text_input_mode_t::CEF_TEXT_INPUT_MODE_NONE;
            self.shared.editable.store(editable, Ordering::Relaxed);
            self.shared.request_repaint();
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&cef::Range>,
            character_bounds: Option<&[cef::Rect]>,
        ) {
            // The text cursor goes after the last character of the composition.
            if let Some(last) = character_bounds.and_then(|bounds| bounds.last()) {
                *self.shared.text_cursor.lock() = Some(Rect::from_min_size(
                    pos2((last.x + last.width) as f32, last.y as f32),
                    egui::vec2(1.0, last.height as f32),
                ));
            }
        }
    }
}

wrap_display_handler! {
    struct EguiDisplayHandler {
        shared: Arc<Shared>,
    }

    impl DisplayHandler {
        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: CursorHandle,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&cef::CursorInfo>,
        ) -> ::std::os::raw::c_int {
            *self.shared.cursor.lock() = cursor_icon(*type_.as_ref());
            self.shared.request_repaint();
            1
        }

        fn on_tooltip(
            &self,
            _browser: Option<&mut Browser>,
            text: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let text = text.map(|t| t.to_string()).filter(|t| !t.is_empty());
            *self.shared.tooltip.lock() = text;
            1
        }
    }
}

wrap_focus_handler! {
    struct EguiFocusHandler {
        shared: Arc<Shared>,
    }

    impl FocusHandler {
        /// Called when Tab moves the focus past the page's last element (`next` is nonzero), or
        /// Shift-Tab past its first.
        fn on_take_focus(&self, _browser: Option<&mut Browser>, next: ::std::os::raw::c_int) {
            *self.shared.took_focus_away.lock() = Some(if next == 0 {
                FocusDirection::Previous
            } else {
                FocusDirection::Next
            });
            self.shared.request_repaint();
        }
    }
}

wrap_load_handler! {
    struct EguiLoadHandler {
        shared: Arc<Shared>,
    }

    impl LoadHandler {
        fn on_load_start(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: cef::TransitionType,
        ) {
            if let Some(url) = main_frame_url(frame) {
                self.shared.send(WebViewEvent::Loading { url });
            }
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            _http_status_code: ::std::os::raw::c_int,
        ) {
            if let Some(url) = main_frame_url(frame) {
                self.shared.send(WebViewEvent::Loaded { url });
            }
        }
    }
}

/// The URL of `frame`, if it is the main frame.
fn main_frame_url(frame: Option<&mut cef::Frame>) -> Option<String> {
    let frame = frame.filter(|frame| frame.is_main() == 1)?;
    Some(CefString::from(&frame.url()).to_string())
}

wrap_life_span_handler! {
    struct EguiLifeSpanHandler {}

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

/// A context menu that CEF asked to show, which egui shows instead.
pub struct ContextMenu {
    /// Where the page asked for the menu, in view points. It becomes the menu's top-left corner.
    pub pos: Pos2,
    pub items: Vec<MenuItem>,

    /// Receives the ID of the command the user picked (`cont`), or `cancel` when the menu closes
    /// without a pick. One of the two must be called.
    pub callback: cef::RunContextMenuCallback,

    /// Whether egui has shown the menu yet. Its first pass is a sizing pass, because its
    /// [`egui::Area`] still remembers the size of the previous menu.
    pub shown: bool,
}

/// An item of a [`ContextMenu`].
pub enum MenuItem {
    Command {
        id: i32,
        label: String,
        enabled: bool,
        checked: bool,
    },
    Separator,
}

wrap_context_menu_handler! {
    struct EguiContextMenuHandler {
        shared: Arc<Shared>,
    }

    impl ContextMenuHandler {
        /// Removes the Print and View Source items, which a windowless browser doesn't support.
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
        ) {
            if let Some(model) = model {
                for id in [sys::cef_menu_id_t::MENU_ID_PRINT, sys::cef_menu_id_t::MENU_ID_VIEW_SOURCE] {
                    model.remove(id as i32);
                }
            }
        }

        /// Hands the menu to egui, because a windowless browser can't show CEF's native context
        /// menu. A menu that is still open is cancelled.
        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
            callback: Option<&mut cef::RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(params), Some(model), Some(callback)) = (params, model, callback) else {
                return 0;
            };
            #[expect(clippy::cast_precision_loss)] // View points are far below 2^24.
            let pos = pos2(params.xcoord() as f32, params.ycoord() as f32);
            let menu = ContextMenu {
                pos,
                items: menu_items(model),
                callback: callback.clone(),
                shown: false,
            };
            if let Some(old) = self.shared.context_menu.lock().replace(menu) {
                old.callback.cancel();
            }
            self.shared.request_repaint();
            1
        }
    }
}

/// The items of `model`, without leading, trailing or repeated separators.
///
/// Submenus are left out, because this backend doesn't show them.
fn menu_items(model: &cef::MenuModel) -> Vec<MenuItem> {
    use sys::cef_menu_item_type_t as T;
    let mut items = Vec::new();
    for index in 0..model.count() {
        let item = match *model.type_at(index).as_ref() {
            T::MENUITEMTYPE_SEPARATOR => MenuItem::Separator,
            T::MENUITEMTYPE_SUBMENU | T::MENUITEMTYPE_NONE => continue,
            _ => MenuItem::Command {
                id: model.command_id_at(index),
                label: menu_label(&CefString::from(&model.label_at(index)).to_string()),
                enabled: model.is_enabled_at(index) == 1,
                checked: model.is_checked_at(index) == 1,
            },
        };
        // Skip a separator at the start or right after another one.
        let after_separator = items
            .last()
            .is_none_or(|last| matches!(last, MenuItem::Separator));
        if !(matches!(item, MenuItem::Separator) && after_separator) {
            items.push(item);
        }
    }
    if matches!(items.last(), Some(MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// `label` without the `&` that marks its access key. `&&` stands for a literal `&`.
fn menu_label(label: &str) -> String {
    let mut text = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            text.extend(chars.next());
        } else {
            text.push(c);
        }
    }
    text
}

wrap_client! {
    struct EguiClient {
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
