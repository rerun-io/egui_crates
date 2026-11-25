//! The parts of the macOS and Windows backends that are the same for both: a wry webview and
//! the events it reports.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::Context;
use raw_window_handle::HasWindowHandle;

use crate::{Error, WebViewEvent, WebViewSource, warn};

/// A wry webview, together with the [`WebViewEvent`]s it reported since the last
/// [`Self::take_events`].
///
/// Each wry backend owns one and adds its platform's placement and focus handling around it.
pub struct WryPage {
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
    /// The webview starts hidden unless `visible` is `true`.
    ///
    /// # Errors
    /// If wry fails to create the webview.
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        parent: &impl HasWindowHandle,
        visible: bool,
    ) -> Result<Self, Error> {
        check_url(source)?;
        let events = Rc::new(RefCell::new(Vec::new()));
        let builder = wry::WebViewBuilder::new()
            .with_visible(visible)
            // Otherwise wry activates the app (macOS) or focuses the page (Windows) when it
            // creates the webview.
            .with_focused(false)
            .with_ipc_handler({
                let (ctx, events) = (ctx.clone(), Rc::clone(&events));
                move |request| {
                    events
                        .borrow_mut()
                        .push(WebViewEvent::Message(request.into_body()));
                    ctx.request_repaint();
                }
            })
            .with_on_page_load_handler({
                let (ctx, events) = (ctx.clone(), Rc::clone(&events));
                move |event, url| {
                    events.borrow_mut().push(match event {
                        wry::PageLoadEvent::Started => WebViewEvent::Loading { url },
                        wry::PageLoadEvent::Finished => WebViewEvent::Loaded { url },
                    });
                    ctx.request_repaint();
                }
            });
        let builder = match source {
            WebViewSource::Html(html) => builder.with_html(html),
            WebViewSource::Url(url) => builder.with_url(url),
        };
        let webview = builder.build_as_child(parent).map_err(Error::backend)?;
        Ok(Self {
            webview,
            events,
            zoom: Cell::new(1.0),
        })
    }

    pub fn load(&self, source: &WebViewSource) -> Result<(), Error> {
        check_url(source)?;
        match source {
            WebViewSource::Html(html) => self.webview.load_html(html),
            WebViewSource::Url(url) => self.webview.load_url(url),
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

/// Prevent wry from panicking when `NSURL` cannot parse a URL.
fn check_url(source: &WebViewSource) -> Result<(), Error> {
    cfg_select! {
        target_os = "macos" => {
            use objc2_foundation::{NSString, NSURL};

            if let WebViewSource::Url(url) = source
                && NSURL::URLWithString(&NSString::from_str(url)).is_none()
            {
                return Err(Error::backend(format!("Not a URL: {url:?}")));
            }
            Ok(())
        }
        _ => {
            _ = source;
            Ok(())
        }
    }
}
