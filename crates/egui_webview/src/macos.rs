//! The macOS backend displays a wry `WKWebView` below egui's surface.
//!
//! egui clears the webview's visible rect to transparent. Its Metal layer retains alpha, so
//! the page shows through while egui windows and popups can still draw over it.
//!
//! The webview is a subview of winit's view. `ClipView` clips both drawing and hit-testing to
//! the visible area, and passes clicks in egui's input areas through to winit.
//! `FocusEdge` views on either side of the webview handle Tab navigation into and out of it.

#![expect(unsafe_code)] // AppKit

mod clip_view;
mod focus_edge;
mod focus_observer;

use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use egui::{Context, EmbeddedViewOutput, FocusDirection, Pos2, Rect, Theme};
use objc2::rc::Retained;
use objc2::{DefinedClass as _, MainThreadMarker};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization as _, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSAutoresizingMaskOptions, NSResponder, NSView,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_quartz_core::CAMetalLayer;
use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle};
use wry::WebViewExtMacOS as _;

use self::clip_view::ClipView;
use self::focus_edge::FocusEdge;
use self::focus_observer::FocusObserver;
use crate::wry_page::WryPage;
use crate::{Backend, Error, WebViewEvent, WebViewSource};

/// The output last applied to the views.
#[derive(Debug, PartialEq)]
struct Placement {
    output: EmbeddedViewOutput,

    /// egui's zoom factor when `output` was applied.
    zoom: f32,
}

/// Returns winit's `NSView` of `window`.
///
/// # Errors
/// If `window` has no `AppKit` window handle, or if egui doesn't render with Metal, since the
/// pages would then cover egui.
fn window_view(window: &impl HasWindowHandle) -> Result<Retained<NSView>, Error> {
    let handle = window.window_handle().map_err(Error::backend)?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(Error::Unsupported);
    };
    // SAFETY: The handle points to an `NSView` that is alive while the handle is borrowed.
    // Retaining the view keeps it alive afterwards.
    let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
        .ok_or(Error::Unsupported)?;

    // egui's Metal layer is a sublayer of the view's layer, so the pages can go below it.
    // OpenGL draws into the view's backing layer, below all sublayers.
    // SAFETY: `sublayers` returns an array of `CALayer`s, or nil.
    let has_metal_layer =
        unsafe { view.layer().and_then(|layer| layer.sublayers()) }.is_some_and(|layers| {
            layers
                .iter()
                .any(|layer| layer.downcast::<CAMetalLayer>().is_ok())
        });
    if has_metal_layer {
        Ok(view)
    } else {
        Err(Error::backend(
            "egui doesn't render with Metal, so the page would cover egui. \
             Use eframe's wgpu renderer.",
        ))
    }
}

/// Exposes an `NSView` as a window handle, so that wry adds its `WKWebView` as a subview of it.
struct ViewHandle<'a>(&'a NSView);

impl HasWindowHandle for ViewHandle<'_> {
    fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, HandleError> {
        let handle = AppKitWindowHandle::new(NonNull::from(self.0).cast());
        // SAFETY: `self` borrows the view, so the view outlives the returned handle.
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(RawWindowHandle::AppKit(handle)) })
    }
}

/// Shows a page in a `WKWebView` below egui's surface.
#[derive(Debug)]
pub struct WryBackend {
    page: WryPage,
    clip: Retained<ClipView>,

    /// `None` while hidden, which is how the page starts.
    last: Option<Placement>,

    /// The [`FocusEdge`]s before and after the `WKWebView` in the key view loop, in that order.
    focus_edges: [Retained<FocusEdge>; 2],

    /// Keeps `AppKit` focus notifications active even while egui is idle.
    _focus_observer: Retained<FocusObserver>,

    /// The direction in which the keyboard focus left the page, set by a [`FocusEdge`].
    focus_left: Rc<Cell<Option<FocusDirection>>>,

    /// A [`Backend::focus`] deferred until the page is shown, because `AppKit` skips hidden
    /// views in the key view loop.
    ///
    /// Also set when a focused page is hidden, so that it gets the focus back when shown again.
    pending_focus: Option<FocusDirection>,
}

impl WryBackend {
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        window: &impl HasWindowHandle,
    ) -> Result<Self, Error> {
        let mtm = MainThreadMarker::new()
            .ok_or_else(|| Error::backend("webviews have to be made on the main thread"))?;
        let parent = window_view(window)?;

        let clip = ClipView::new(mtm);
        clip.setWantsLayer(true);
        if let Some(layer) = clip.layer() {
            // Apps linked against the macOS 14 SDK or later don't clip subviews to their
            // superview's bounds by default.
            layer.setMasksToBounds(true);
        }
        clip.setHidden(true);
        parent.addSubview(&clip);

        let page = match WryPage::new(ctx, source, &ViewHandle(&clip)) {
            Ok(page) => page,
            Err(err) => {
                clip.removeFromSuperview();
                return Err(err);
            }
        };
        let view = page.webview.webview();
        // wry sets an autoresizing mask that pins a child webview to the top of its parent.
        // Clear it, because `place` sets the frame.
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewNotSizable);

        let focus_left = Rc::new(Cell::new(None));
        let focus_edges = [FocusDirection::Previous, FocusDirection::Next]
            .map(|direction| FocusEdge::new(mtm, ctx, direction, &focus_left));
        let [before, after] = &focus_edges;
        clip.addSubview(before);
        clip.addSubview(after);
        // SAFETY: The key view loop doesn't retain its views. `clip` retains all three as
        // subviews for as long as it lives.
        unsafe {
            before.setNextKeyView(Some(&view));
            view.setNextKeyView(Some(after));
        }

        let focus_observer = FocusObserver::new(ctx, page.webview.ns_window(), &view);
        Ok(Self {
            page,
            clip,
            last: None,
            focus_edges,
            _focus_observer: focus_observer,
            focus_left,
            pending_focus: None,
        })
    }

    /// The window's first responder, if it is a view.
    fn first_responder_view(&self) -> Option<Retained<NSView>> {
        self.clip
            .window()?
            .firstResponder()?
            .downcast::<NSView>()
            .ok()
    }

    /// Whether the window's first responder is the webview or one of its descendants.
    fn view_focused(&self) -> bool {
        self.first_responder_view()
            .is_some_and(|view| view.isDescendantOf(&self.page.webview.webview()))
    }

    /// Whether the window's first responder is the webview, one of its descendants, or a
    /// focus edge, which briefly holds the focus after Tab leaves the page.
    fn view_or_edge_focused(&self) -> bool {
        self.first_responder_view().is_some_and(|view| {
            view.isDescendantOf(&self.page.webview.webview())
                || self.focus_edges.iter().any(|edge| *view == ***edge)
        })
    }
}

impl Backend for WryBackend {
    fn name(&self) -> &'static str {
        "wry (WKWebView)"
    }

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

    fn can_go_back(&self) -> bool {
        self.page.can_go_back()
    }

    fn can_go_forward(&self) -> bool {
        self.page.can_go_forward()
    }

    fn set_theme(&mut self, theme: Theme) {
        // `WebKit` gives the page the view's appearance as its `prefers-color-scheme`.
        // SAFETY: `AppKit` defines these constants, and they never change.
        let name = unsafe {
            match theme {
                Theme::Dark => NSAppearanceNameDarkAqua,
                Theme::Light => NSAppearanceNameAqua,
            }
        };
        let appearance = NSAppearance::appearanceNamed(name);
        self.page
            .webview
            .webview()
            .setAppearance(appearance.as_deref());
    }

    fn place(&mut self, ctx: &Context, output: Option<&EmbeddedViewOutput>) {
        // `AppKit` places views in window points. One ui point is `zoom` window points.
        let zoom = ctx.zoom_factor();
        self.page.set_zoom(zoom);
        let placement = output.map(|output| Placement {
            output: output.clone(),
            zoom,
        });
        if self.last == placement {
            return;
        }
        self.last = placement;

        let Some(output) = output else {
            // A hidden view stays first responder and keeps receiving key events. Give the
            // focus to egui, and give it back to the page when the page is shown again.
            if self.view_or_edge_focused() {
                self.blur();
                self.pending_focus = Some(FocusDirection::None);
            }
            self.clip.setHidden(true);
            return;
        };

        let rect = output.rect * zoom;
        let visible = output.visible * zoom;
        let origin = visible.min.to_vec2();
        let takes_input = !output.view_input_areas().is_empty();
        *self.clip.ivars().egui_input_areas.borrow_mut() = if takes_input {
            output
                .egui_input_areas
                .iter()
                .map(|area| (*area * zoom).translate(-origin))
                .collect()
        } else {
            // egui takes the pointer everywhere, as under a modal. One area that covers the
            // clip view replaces the many that `egui_input_areas` may contain.
            vec![Rect::from_min_size(Pos2::ZERO, visible.size())]
        };

        // These changes commit in the implicit Core Animation transaction, in which eframe
        // also presents egui's frame, so don't open an explicit one. Layers that back views
        // don't animate property changes.
        self.clip.setFrame(cg_rect(visible));
        self.page
            .webview
            .webview()
            .setFrame(cg_rect(rect.translate(-origin)));
        if let Some(layer) = self.clip.layer() {
            #[expect(clippy::cast_precision_loss)]
            // Higher indices approach egui's Metal layer at zero from below.
            layer.setZPosition(-1.0 / (output.z_index as f64 + 1.0));
        }

        self.clip.setHidden(false);
        if let Some(direction) = self.pending_focus.take() {
            self.focus(direction);
        }
    }

    fn focus(&mut self, direction: FocusDirection) {
        if self.clip.isHidden() {
            self.pending_focus = Some(direction);
            return;
        }
        let Some(window) = self.clip.window() else {
            return;
        };
        let [before, after] = &self.focus_edges;
        // Moving forward through the key view loop into the `WKWebView` makes `WebKit` focus
        // the page's first element, and moving backward its last element. Making it first
        // responder directly keeps the element the page had focused.
        match direction {
            FocusDirection::Next => window.selectKeyViewFollowingView(before),
            FocusDirection::Previous => window.selectKeyViewPrecedingView(after),
            _ => {
                window.makeFirstResponder(Some(&self.page.webview.webview()));
            }
        }
    }

    fn blur(&mut self) {
        self.pending_focus = None;
        if self.view_or_edge_focused()
            && let Some(window) = self.clip.window()
        {
            // SAFETY: `superview` returns an `NSView`, or nil.
            let egui_view = unsafe { self.clip.superview() };
            window.makeFirstResponder(egui_view.as_deref().map(|view| -> &NSResponder { view }));
        }
    }

    fn has_focus(&self) -> bool {
        self.view_focused()
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
        // When the first responder is removed from its window, `AppKit` makes the window
        // itself first responder, and winit's view then gets no key events. Give the focus to
        // winit's view first.
        self.blur();
        self.clip.removeFromSuperview();
    }
}

fn cg_rect(rect: Rect) -> CGRect {
    CGRect::new(
        CGPoint::new(f64::from(rect.min.x), f64::from(rect.min.y)),
        CGSize::new(f64::from(rect.width()), f64::from(rect.height())),
    )
}
