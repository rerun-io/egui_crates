//! Off-screen rendering: the view's size and scale, the frames CEF paints, the popup widget, and
//! the IME.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{Browser, ImplRenderHandler, RenderHandler, WrapRenderHandler, sys, wrap_render_handler};
use egui::{Rect, pos2};

use crate::chromium::context::{Context, PopupPlacement};
use crate::chromium::layer::Layer;

wrap_render_handler! {
    pub struct EguiRenderHandler {
        context: Arc<Context>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut cef::Rect>) {
            if let Some(rect) = rect {
                let size = self.context.state.lock().size.0.round();
                // CEF requires a view rect that isn't empty.
                rect.x = 0;
                rect.y = 0;
                #[expect(clippy::cast_possible_truncation)] // CEF takes whole points.
                let (width, height) = (size.x as i32, size.y as i32);
                rect.width = width.max(1);
                rect.height = height.max(1);
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
            screen_info.device_scale_factor = self.context.state.lock().size.1;
            1
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: ::std::os::raw::c_int) {
            if show == 0 {
                {
                    let mut state = self.context.state.lock();
                    state.popup_placement = None;
                    state.popup = Layer::default();
                }
                self.context.request_repaint();
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&cef::Rect>) {
            let Some(rect) = rect else {
                return;
            };
            // Move the popup inside the view, like cefclient does.
            let mut state = self.context.state.lock();
            let (size, _) = state.size;
            let popup_size = egui::vec2(rect.width as f32, rect.height as f32);
            let requested = Rect::from_min_size(pos2(rect.x as f32, rect.y as f32), popup_size);
            state.popup_placement = Some(PopupPlacement::new(size, requested));
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
            {
                let mut state = self.context.state.lock();
                let layer = if *type_.as_ref() == sys::cef_paint_element_type_t::PET_POPUP {
                    &mut state.popup
                } else {
                    &mut state.view
                };
                layer.paint(buffer, [width, height], dirty_rects.unwrap_or_default());
            }
            self.context.request_repaint();
        }

        fn on_virtual_keyboard_requested(
            &self,
            _browser: Option<&mut Browser>,
            input_mode: cef::TextInputMode,
        ) {
            // CEF asks for a virtual keyboard when an editable element gains focus, and passes
            // `CEF_TEXT_INPUT_MODE_NONE` when it loses it.
            let editable = *input_mode.as_ref() != sys::cef_text_input_mode_t::CEF_TEXT_INPUT_MODE_NONE;
            self.context.state.lock().keyboard_requested = editable;
            self.context.request_repaint();
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            _selected_range: Option<&cef::Range>,
            character_bounds: Option<&[cef::Rect]>,
        ) {
            // The text cursor goes after the last character of the composition.
            if let Some(last) = character_bounds.and_then(|bounds| bounds.last()) {
                self.context.state.lock().text_cursor = Some(Rect::from_min_size(
                    pos2((last.x + last.width) as f32, last.y as f32),
                    egui::vec2(1.0, last.height as f32),
                ));
            }
        }
    }
}
