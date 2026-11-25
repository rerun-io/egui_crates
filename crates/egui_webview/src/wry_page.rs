//! A wry webview and the events it reports, shared by the macOS and Windows backends.

#[cfg(target_os = "macos")]
mod ipc;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::Context;
use raw_window_handle::HasWindowHandle;

use crate::{Error, WebViewEvent, WebViewSource, reported_url, warn};

/// A wry webview, together with the [`WebViewEvent`]s it reported since the last
/// [`Self::take_events`].
///
/// The backend that owns it adds the placement and focus handling around it.
pub struct WryPage {
    #[cfg(target_os = "macos")]
    _ipc: ipc::Ipc,
    pub webview: wry::WebView,
    events: Rc<RefCell<Vec<WebViewEvent>>>,

    /// The page zoom last set with [`Self::set_zoom`].
    zoom: Cell<f32>,
}

impl std::fmt::Debug for WryPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WryPage")
            .field("events", &self.events)
            .field("zoom", &self.zoom)
            .finish_non_exhaustive()
    }
}

impl WryPage {
    /// Creates a webview that loads `source`, as a child of `parent`.
    ///
    /// On Windows, the webview starts hidden, since it is a child window that would otherwise
    /// show at its default bounds until the first placement.
    ///
    /// # Errors
    /// If `source` is a URL that `NSURL` can't parse (macOS), or if wry fails to create the
    /// webview.
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        parent: &impl HasWindowHandle,
    ) -> Result<Self, Error> {
        let events = Rc::new(RefCell::new(Vec::new()));
        let builder = wry::WebViewBuilder::new()
            // Otherwise wry activates the app (macOS) or focuses the page (Windows) when it
            // creates the webview.
            .with_focused(false)
            .with_on_page_load_handler({
                let (ctx, events) = (ctx.clone(), Rc::clone(&events));
                move |event, url| {
                    let Some(url) = reported_url(&url) else {
                        return;
                    };
                    events.borrow_mut().push(match event {
                        wry::PageLoadEvent::Started => WebViewEvent::Loading { url },
                        wry::PageLoadEvent::Finished => WebViewEvent::Loaded { url },
                    });
                    ctx.request_repaint();
                }
            });

        let builder = cfg_select! {
            target_os = "macos" => {
                builder.with_initialization_script(ipc::SCRIPT)
            }
            _ => {
                // `WebView2` only raises `WebMessageReceived` for messages from the top-level
                // document, so wry's handler already accepts IPC only from the main frame.
                builder.with_visible(false).with_ipc_handler({
                    let (ctx, events) = (ctx.clone(), Rc::clone(&events));
                    move |request| {
                        events
                            .borrow_mut()
                            .push(WebViewEvent::Message(request.into_body()));
                        ctx.request_repaint();
                    }
                })
            }
        };

        let webview = builder.build_as_child(parent).map_err(Error::backend)?;

        // Register the native handler before the first document can run its scripts.
        #[cfg(target_os = "macos")]
        let ipc = {
            use wry::WebViewExtMacOS as _;
            ipc::Ipc::new(ctx, webview.manager(), Rc::clone(&events))
        };

        let page = Self {
            #[cfg(target_os = "macos")]
            _ipc: ipc,
            webview,
            events,
            zoom: Cell::new(1.0),
        };
        page.load(source)?;
        Ok(page)
    }

    pub fn load(&self, source: &WebViewSource) -> Result<(), Error> {
        check_url(source)?;
        match source {
            WebViewSource::Html(html) => self.webview.load_html(html),
            WebViewSource::Url(url) => self.webview.load_url(url.as_str()),
        }
        .map_err(Error::backend)
    }

    pub fn evaluate_script(&self, script: &str) -> Result<(), Error> {
        self.webview.evaluate_script(script).map_err(Error::backend)
    }

    /// Zooms the page by `zoom`, so that it scales together with egui's
    /// [`Context::zoom_factor`].
    pub fn set_zoom(&self, zoom: f32) {
        if self.zoom.get() != zoom {
            self.zoom.set(zoom);
            warn(self.webview.zoom(zoom.into()));
        }
    }

    pub fn reload(&self) {
        warn(self.webview.reload());
    }

    pub fn back(&self) {
        warn(self.webview.go_back());
    }

    pub fn forward(&self) {
        warn(self.webview.go_forward());
    }

    pub fn take_events(&self) -> Vec<WebViewEvent> {
        self.events.take()
    }
}

/// Returns an error for a URL that `NSURL` can't parse, on which wry panics.
///
/// `NSURL` and the `url` crate follow different standards, so a parsed [`crate::Url`] isn't enough.
/// `WebView2` takes any URL that the `url` crate parses, so elsewhere this always succeeds.
#[cfg_attr(not(target_os = "macos"), expect(clippy::unnecessary_wraps))]
fn check_url(source: &WebViewSource) -> Result<(), Error> {
    cfg_select! {
        target_os = "macos" => {
            use objc2_foundation::{NSString, NSURL};

            if let WebViewSource::Url(url) = source
                && NSURL::URLWithString(&NSString::from_str(url.as_str())).is_none()
            {
                return Err(Error::backend(format!("NSURL can't parse {url}")));
            }
            Ok(())
        }
        _ => {
            _ = source;
            Ok(())
        }
    }
}
