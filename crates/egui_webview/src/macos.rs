//! The macOS backend: a `WKWebView`, through wry, shown below egui's surface.
//!
//! egui clears the visible rect of each webview to transparent ([`egui::Shape::Clear`]). The
//! page's layer is below egui's `CAMetalLayer`, so the page shows through egui's surface, and
//! the egui layers above the webview, such as windows and popups, blend over it. This requires
//! a transparent window; otherwise Core Animation composites egui's layer as opaque.
//!
//! Only the page's *layer* is below egui. Its *view* is a subview of winit's view, so `AppKit`
//! hit-tests it before winit's view. A [`ClipView`] contains the `WKWebView` and clips it to
//! the visible part of its `Ui`, both for drawing and for hit-testing, so a page in a scroll
//! area neither shows nor takes clicks outside the scroll area. In egui's input areas
//! ([`Placement::egui_input_areas`]), the page still draws under egui, but the clip view's
//! hit-test returns `nil`, so `AppKit` delivers those clicks to winit's view and thus to egui.
//!
//! The keyboard focus follows `AppKit`'s key view loop. Two empty [`FocusEdge`] views, before
//! and after the `WKWebView` in the loop, make `WebKit` focus the page's first or last element
//! when Tab enters the page, and detect when Tab leaves it.

#![expect(unsafe_code)] // AppKit

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use egui::{Context, FocusDirection, Pos2, Rect};
use objc2::rc::Retained;
use objc2::{DefinedClass as _, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSResponder, NSView};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_foundation::{NSString, NSURL};
use objc2_quartz_core::CAMetalLayer;
use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle};
use wry::WebViewExtMacOS as _;

use crate::wry_page::{WryPage, log_error};
use crate::{Backend, Error, Placement, WebViewEvent, WebViewSource};

/// The offset that puts every page's layer below egui's `CAMetalLayer`, whose `zPosition` is 0.
///
/// A clip view's layer gets [`Placement::z_index`] minus this offset as its `zPosition`. The
/// pages thus stack in egui's paint order among each other, and stay below egui's layer as long
/// as egui has fewer than 1000 layers.
const BELOW_EGUI: f64 = 1000.0;

/// Returns winit's `NSView` of `window`, with egui's Metal layer set up for pages below it.
///
/// Logs a warning if the window is opaque or egui doesn't render with Metal, since the pages
/// then don't show correctly.
///
/// # Errors
/// If `window` has no `AppKit` window handle.
fn window_view(window: &impl HasWindowHandle) -> Result<Retained<NSView>, Error> {
    let handle = window.window_handle().map_err(Error::backend)?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(Error::Unsupported);
    };
    // SAFETY: The handle points to an `NSView` that is alive while the handle is borrowed.
    // Retaining the view keeps it alive afterwards.
    let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
        .ok_or(Error::Unsupported)?;

    if view.window().is_some_and(|window| window.isOpaque()) {
        log::warn!(
            "egui_webview: the window isn't transparent, so egui will paint over the pages. \
             Use `ViewportBuilder::with_transparent(true)`."
        );
    }
    if let Some(metal) = metal_layer(&view) {
        // Present egui's frames in the Core Animation transaction that also commits the frame
        // changes `place` makes to the page views. Otherwise egui's frame appears after the
        // page has moved, and the window background shows through the rect that the previous
        // frame cleared.
        metal.setPresentsWithTransaction(true);
    } else {
        log::warn!(
            "egui_webview: egui doesn't draw with Metal, so the pages will cover egui. \
             Use eframe's wgpu renderer."
        );
    }
    Ok(view)
}

/// Returns the `CAMetalLayer` sublayer of winit's `view` that egui renders into.
///
/// `None` when egui renders with OpenGL, which draws into the view's own backing layer. That
/// layer is below all sublayers, so the pages can't go below it.
fn metal_layer(view: &NSView) -> Option<Retained<CAMetalLayer>> {
    // SAFETY: `sublayers` returns an array of `CALayer`s, or nil.
    let sublayers = unsafe { view.layer()?.sublayers() }?;
    sublayers
        .iter()
        .find_map(|sublayer| sublayer.downcast::<CAMetalLayer>().ok())
}

#[derive(Default)]
struct ClipIvars {
    /// The areas where egui takes the pointer, in the clip view's own (flipped) coordinates.
    ///
    /// The clip view's hit-test returns `nil` inside them.
    egui_input_areas: RefCell<Vec<Rect>>,
}

define_class!(
    /// A view that contains a `WKWebView` and clips it to [`Placement::visible`].
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewClipView"]
    #[ivars = ClipIvars]
    struct ClipView;

    impl ClipView {
        /// Uses a top-left origin, like winit's view and egui.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Returns `nil` for a point in egui's input areas, so `AppKit` delivers the click to
        /// winit's view, and thus to egui.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: CGPoint) -> Option<Retained<NSView>> {
            // `hitTest:` gets `point` in the superview's coordinates.
            // SAFETY: `superview` returns an `NSView`, or nil.
            let superview = unsafe { self.superview() };
            let local = self.convertPoint_fromView(point, superview.as_deref());
            #[expect(clippy::cast_possible_truncation)]
            let local = egui::pos2(local.x as f32, local.y as f32);
            if self.ivars().egui_input_areas.borrow().iter().any(|area| area.contains(local)) {
                None
            } else {
                // SAFETY: The superclass method has the same signature.
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    }
);

impl ClipView {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClipIvars::default());
        // SAFETY: `NSView` supports `init`, which calls `initWithFrame:` with a zero rect.
        unsafe { msg_send![super(this), init] }
    }
}

struct FocusEdgeIvars {
    /// The direction in which the focus left the page when this view becomes first responder.
    direction: FocusDirection,

    /// Shared with [`WryBackend::focus_left`]. Set to `direction` when this view becomes first
    /// responder.
    left: Rc<Cell<Option<FocusDirection>>>,

    /// Repainted when this view becomes first responder, so that the webview polls
    /// [`Backend::poll_focus_left`].
    ctx: Context,
}

define_class!(
    /// An empty view just before or after the `WKWebView` in the window's key view loop.
    ///
    /// When Tab leaves the page's last element, `WebKit` makes the next key view first
    /// responder, and the previous one for Shift-Tab. A focus edge that becomes first responder
    /// thus reports that the focus left the page, and in which direction.
    ///
    /// Conversely, selecting the key view that follows the edge before the page makes `WebKit`
    /// focus the page's first element, and selecting the one that precedes the edge after the
    /// page focuses its last element.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "EguiWebViewFocusEdge"]
    #[ivars = FocusEdgeIvars]
    struct FocusEdge;

    impl FocusEdge {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// Joins the key view loop even when "Keyboard navigation" is off in System Settings.
        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ivars = self.ivars();
            ivars.left.set(Some(ivars.direction));
            ivars.ctx.request_repaint();
            true
        }
    }
);

impl FocusEdge {
    fn new(
        mtm: MainThreadMarker,
        ctx: &Context,
        direction: FocusDirection,
        left: &Rc<Cell<Option<FocusDirection>>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FocusEdgeIvars {
            direction,
            left: Rc::clone(left),
            ctx: ctx.clone(),
        });
        // SAFETY: `NSView` supports `init`, which calls `initWithFrame:` with a zero rect.
        unsafe { msg_send![super(this), init] }
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

/// Shows a page in a `WKWebView` below egui's surface. See the module docs.
pub struct WryBackend {
    page: WryPage,
    clip: Retained<ClipView>,

    /// The HTML of the last loaded source, if it was [`WebViewSource::Html`].
    ///
    /// [`Backend::reload`] loads it again.
    html: Option<String>,

    /// The placement last applied to the views. `None` while hidden, which is how the page
    /// starts.
    last: Option<Placement>,

    /// The [`FocusEdge`]s before and after the `WKWebView` in the key view loop, in that order.
    focus_edges: [Retained<FocusEdge>; 2],

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
        check_url(source)?;
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

        let page = match WryPage::new(ctx, source, &ViewHandle(&clip), true) {
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

        Ok(Self {
            page,
            clip,
            html: source.html().map(str::to_owned),
            last: None,
            focus_edges,
            focus_left,
            pending_focus: None,
        })
    }
}

impl WryBackend {
    /// Whether the window's first responder is the `WKWebView` or one of its descendants.
    ///
    /// With `or_edge`, also whether it is one of the [`FocusEdge`]s, which briefly hold the
    /// focus after Tab leaves the page.
    fn is_focused(&self, or_edge: bool) -> bool {
        let Some(responder) = self
            .clip
            .window()
            .and_then(|window| window.firstResponder())
        else {
            return false;
        };
        let Ok(view) = responder.downcast::<NSView>() else {
            return false;
        };
        view.isDescendantOf(&self.page.webview.webview())
            || (or_edge && self.focus_edges.iter().any(|edge| *view == ***edge))
    }
}

impl Backend for WryBackend {
    fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
        check_url(source)?;
        self.page.load(source)?;
        self.html = source.html().map(str::to_owned);
        Ok(())
    }

    fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
        self.page.evaluate_script(script)
    }

    fn reload(&mut self) {
        // `WebKit` reloads a page that was loaded from an HTML string as an empty
        // `about:blank`, so load the HTML again instead.
        // SAFETY: `URL` is a getter whose only requirement is the main thread, which every
        // call on the webview runs on.
        let url = unsafe { self.page.webview.webview().URL() }.and_then(|url| url.absoluteString());
        if let Some(html) = &self.html
            && url.is_some_and(|url| url.to_string().split('#').next() == Some("about:blank"))
        {
            log_error(self.page.webview.load_html(html));
        } else {
            self.page.reload();
        }
    }

    fn back(&mut self) {
        self.page.back();
    }

    fn forward(&mut self) {
        self.page.forward();
    }

    fn is_native_view(&self) -> bool {
        true
    }

    fn place(&mut self, placement: Option<&Placement>) {
        let placement = placement.filter(|placement| placement.visible.is_positive());
        if self.last.as_ref() == placement {
            return;
        }
        self.last = placement.cloned();

        let Some(placement) = placement else {
            // A hidden view stays first responder and keeps receiving key events. Give the
            // focus to egui, and give it back to the page when the page is shown again.
            if self.is_focused(true) {
                self.blur();
                self.pending_focus = Some(FocusDirection::None);
            }
            self.clip.setHidden(true);
            return;
        };

        let origin = placement.visible.min.to_vec2();
        *self.clip.ivars().egui_input_areas.borrow_mut() = if placement.takes_input {
            placement
                .egui_input_areas
                .iter()
                .map(|area| area.translate(-origin))
                .collect()
        } else {
            // egui takes the pointer everywhere, as under a modal. One area that covers the
            // clip view replaces the many that `egui_input_areas` may contain.
            vec![Rect::from_min_size(Pos2::ZERO, placement.visible.size())]
        };

        // These changes commit in the implicit Core Animation transaction that also presents
        // egui's frame (see `window_view`), so don't open an explicit one. Layers that back
        // views don't animate property changes.
        self.clip.setFrame(cg_rect(placement.visible));
        self.page
            .webview
            .webview()
            .setFrame(cg_rect(placement.rect.translate(-origin)));
        if let Some(layer) = self.clip.layer() {
            #[expect(clippy::cast_precision_loss)]
            layer.setZPosition(placement.z_index as f64 - BELOW_EGUI);
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
        if self.is_focused(true)
            && let Some(window) = self.clip.window()
        {
            // SAFETY: `superview` returns an `NSView`, or nil.
            let egui_view = unsafe { self.clip.superview() };
            window.makeFirstResponder(egui_view.as_deref().map(|view| -> &NSResponder { view }));
        }
    }

    fn has_focus(&self) -> bool {
        self.is_focused(false)
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

/// Returns an error for a URL that `NSURL` can't parse, such as `http://[::1`, on which wry
/// panics.
fn check_url(source: &WebViewSource) -> Result<(), Error> {
    let WebViewSource::Url(url) = source else {
        return Ok(());
    };
    if NSURL::URLWithString(&NSString::from_str(url)).is_some() {
        Ok(())
    } else {
        Err(Error::backend(format!("Not a URL: {url:?}")))
    }
}
