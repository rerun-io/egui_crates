//! The Windows backend: a `WebView2`, created through wry, below egui's surface.
//!
//! wry shows the page in a child window of egui's window. Child windows are drawn over their
//! parent's client area, so egui presents into a `DirectComposition` visual that wgpu places
//! above them ([`wgpu_configuration`]). egui clears the visible rect of each webview to
//! transparent ([`egui::Shape::Clear`]), so the page shows through egui's surface, and the egui
//! layers above the webview, such as windows and popups, blend over it.
//!
//! A child window still receives all mouse input inside its rect. Each page's window therefore
//! gets a window region ([`SetWindowRgn`]) that leaves out the areas egui paints over, and the
//! mouse input there goes to egui's window. The region also clips what the page draws. Since
//! the areas are rectangles, the page doesn't show in the transparent rounded corners of egui's
//! windows and menus.
//!
//! `WebView2` can focus the page's first or last element itself ([`MoveFocus`]), and it raises
//! `MoveFocusRequested` when Tab leaves the page.
//!
//! [`MoveFocus`]: webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller::MoveFocus

#![expect(unsafe_code)] // Win32

use std::cell::Cell;
use std::rc::Rc;

use egui::{Context, EmbeddedViewOutput, FocusDirection, Rect};
use raw_window_handle::RawWindowHandle;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MOVE_FOCUS_REASON, COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT,
    COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS, COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
};
use webview2_com::{FocusChangedEventHandler, MoveFocusRequestedEventHandler};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, HRGN, RGN_DIFF, SetWindowRgn,
};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use wry::WebViewExtWindows as _;

use crate::wry_page::WryPage;
use crate::{Backend, Error, WebViewEvent, WebViewSource, warn};

/// Configures wgpu so that egui draws over the pages. Apply it to eframe's
/// `NativeOptions::wgpu_options` before [`crate::WebViewInstance::new`] creates a page.
///
/// Selects wgpu's DX12 backend and makes it present into a `DirectComposition` visual above the
/// window's child windows ([`wgpu::Dx12SwapchainKind::DxgiFromTopmostVisual`]). Does nothing
/// unless `config.wgpu_setup` is [`egui_wgpu::WgpuSetup::CreateNew`].
///
/// The window must also be transparent (`ViewportBuilder::with_transparent(true)`), so that the
/// pages show where egui clears its surface.
pub fn wgpu_configuration(config: &mut egui_wgpu::WgpuConfiguration) {
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup {
        let descriptor = &mut setup.instance_descriptor;
        descriptor.backends = wgpu::Backends::DX12;
        descriptor.backend_options.dx12.presentation_system =
            wgpu::Dx12SwapchainKind::DxgiFromTopmostVisual;
    }
}

/// The parts of an [`EmbeddedViewOutput`] that this backend uses.
///
/// Rects are `[left, top, right, bottom]` in physical pixels, relative to the client area of
/// egui's window.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Placed {
    rect: [i32; 4],
    visible: [i32; 4],
    egui_paint_areas: Vec<[i32; 4]>,
    takes_input: bool,
}

/// Shows the page in a `WebView2` under egui's surface.
pub struct WryBackend {
    page: WryPage,

    /// Whether the `WebView2` controller has the keyboard focus, set by its `GotFocus` and
    /// `LostFocus` events.
    focused: Rc<Cell<bool>>,

    /// The direction in which Tab left the page since the last [`Backend::poll_focus_left`],
    /// set by `MoveFocusRequested`.
    focus_left: Rc<Cell<Option<FocusDirection>>>,

    /// The placement last applied to the page's window. `None` while hidden, which is how the
    /// page starts.
    last: Option<Placed>,

    /// The outer rect of egui's window when last checked, to detect when the window moves.
    last_window_rect: Option<Rect>,
}

impl WryBackend {
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        window: &impl raw_window_handle::HasWindowHandle,
    ) -> Result<Self, Error> {
        let handle = window.window_handle().map_err(Error::backend)?;
        if !matches!(handle.as_raw(), RawWindowHandle::Win32(_)) {
            return Err(Error::Unsupported);
        }

        let page = WryPage::new(ctx, source, &handle, false)?;

        let focused = Rc::new(Cell::new(false));
        let focus_left = Rc::new(Cell::new(None));
        let controller = page.webview.controller();
        let mut token = 0;
        // SAFETY: `controller` is a live `ICoreWebView2Controller`, and `token` is a valid
        // out-pointer for each call. The controller keeps its own reference to each handler.
        unsafe {
            controller
                .add_GotFocus(
                    &FocusChangedEventHandler::create(Box::new({
                        let (ctx, focused) = (ctx.clone(), Rc::clone(&focused));
                        move |_, _| {
                            focused.set(true);
                            ctx.request_repaint();
                            Ok(())
                        }
                    })),
                    &raw mut token,
                )
                .map_err(Error::backend)?;
            controller
                .add_LostFocus(
                    &FocusChangedEventHandler::create(Box::new({
                        let focused = Rc::clone(&focused);
                        move |_, _| {
                            focused.set(false);
                            Ok(())
                        }
                    })),
                    &raw mut token,
                )
                .map_err(Error::backend)?;
            controller
                .add_MoveFocusRequested(
                    &MoveFocusRequestedEventHandler::create(Box::new({
                        let (ctx, focus_left) = (ctx.clone(), Rc::clone(&focus_left));
                        move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON::default();
                            args.Reason(&raw mut reason)?;
                            let direction = if reason == COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS {
                                FocusDirection::Previous
                            } else {
                                FocusDirection::Next
                            };
                            focus_left.set(Some(direction));
                            ctx.request_repaint();
                            // `WebView2` keeps the focus until the webview calls `blur`, which
                            // gives it to egui.
                            args.SetHandled(true)
                        }
                    })),
                    &raw mut token,
                )
                .map_err(Error::backend)?;
        }

        Ok(Self {
            page,
            focused,
            focus_left,
            last: None,
            last_window_rect: None,
        })
    }

    /// Notifies `WebView2` when egui's window has moved, so that the page's popups, such as
    /// `<select>` dropdowns, open at the right screen position.
    fn track_window(&mut self, ctx: &Context) {
        let window_rect = ctx.input(|input| input.viewport().outer_rect);
        if window_rect != self.last_window_rect {
            self.last_window_rect = window_rect;
            // SAFETY: A COM call without arguments on a live controller.
            warn(unsafe {
                self.page
                    .webview
                    .controller()
                    .NotifyParentWindowPositionChanged()
            });
        }
    }

    /// Converts `output` from ui points to physical pixels.
    fn placed(ctx: &Context, output: &EmbeddedViewOutput) -> Placed {
        let scale = ctx.pixels_per_point();
        let pixels = |rect: Rect| {
            #[expect(clippy::cast_possible_truncation)]
            let round = |x: f32| (x * scale).round() as i32;
            [
                round(rect.min.x),
                round(rect.min.y),
                round(rect.max.x),
                round(rect.max.y),
            ]
        };

        Placed {
            rect: pixels(output.rect),
            visible: pixels(output.visible),
            egui_paint_areas: output
                .egui_paint_areas
                .iter()
                .copied()
                .map(pixels)
                .collect(),
            takes_input: !output.view_input_areas().is_empty(),
        }
    }

    fn show(&self, placed: &Placed) {
        let [left, top, right, bottom] = placed.rect;
        warn(self.page.webview.set_bounds(wry::Rect {
            position: wry::dpi::PhysicalPosition::new(left, top).into(),
            size: wry::dpi::PhysicalSize::new(right - left, bottom - top).into(),
        }));

        let hwnd = self.page.webview.hwnd();
        // SAFETY: GDI calls on regions that this block creates. `SetWindowRgn` takes ownership
        // of `region`, and each `area` is deleted after use.
        unsafe {
            let region = rect_region(placed.visible, [left, top]);
            for area in &placed.egui_paint_areas {
                let area = rect_region(*area, [left, top]);
                CombineRgn(Some(region), Some(region), Some(area), RGN_DIFF);
                _ = DeleteObject(area.into());
            }
            SetWindowRgn(hwnd, Some(region), true);

            // The window region clips drawing as well as input, so it can't leave out input
            // alone. Where egui takes all the input, as under a modal, disable the window
            // instead: Windows sends the mouse input of a disabled child window to its parent.
            _ = EnableWindow(hwnd, placed.takes_input);
        }

        warn(self.page.webview.set_visible(true));
    }
}

/// Creates a rectangular region for `rect`, translated so that `origin` becomes (0, 0).
///
/// The region is rectangular even for egui's rounded windows and menus, because the page
/// mustn't take clicks in their transparent corners, which egui handles. An area that is
/// clipped to the page's visible rect also has no rounded corner at the clipped edge.
///
/// # Safety
/// The caller owns the returned region, and must delete it or hand it to `SetWindowRgn`.
unsafe fn rect_region(rect: [i32; 4], origin: [i32; 2]) -> HRGN {
    let [left, top, right, bottom] = rect;
    let [x, y] = origin;
    // SAFETY: `CreateRectRgn` takes only integers.
    unsafe { CreateRectRgn(left - x, top - y, right - x, bottom - y) }
}

impl Backend for WryBackend {
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        self.page.load(source)
    }

    fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        self.page.evaluate_script(script)
    }

    fn reload(&mut self) {
        self.page.reload();
    }

    fn back(&mut self) {
        self.page.back();
    }

    fn forward(&mut self) {
        self.page.forward();
    }

    fn place(&mut self, ctx: &Context, output: Option<&EmbeddedViewOutput>) {
        let placed = output.map(|output| Self::placed(ctx, output));
        if placed.is_some() {
            self.track_window(ctx);
        }
        if self.last == placed {
            return;
        }
        match &placed {
            Some(placed) => self.show(placed),
            None => warn(self.page.webview.set_visible(false)),
        }
        self.last = placed;
    }

    fn focus(&mut self, direction: FocusDirection) {
        let reason = match direction {
            FocusDirection::Next => COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT,
            FocusDirection::Previous => COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS,
            _ => COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC,
        };
        // SAFETY: A COM call with a valid enum value on a live controller.
        warn(unsafe { self.page.webview.controller().MoveFocus(reason) });
    }

    fn blur(&mut self) {
        if self.focused.get() {
            warn(self.page.webview.focus_parent());
            // Clear the flag now: `LostFocus` arrives later, and until then the webview would
            // read the stale `true` as the page taking the focus back.
            self.focused.set(false);
        }
    }

    fn has_focus(&self) -> bool {
        self.focused.get()
    }

    fn poll_focus_left(&mut self) -> Option<FocusDirection> {
        self.focus_left.take()
    }

    fn take_events(&mut self) -> Vec<WebViewEvent> {
        self.page.take_events()
    }
}

impl Drop for WryBackend {
    fn drop(&mut self) {
        // Destroying the focused window leaves no window with the keyboard focus, so give the
        // focus to egui's window first.
        self.blur();
    }
}
