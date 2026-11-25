//! A wry webview and the events it reports.

mod ipc;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::Context;
use objc2_foundation::{NSString, NSURL};
use raw_window_handle::HasWindowHandle;
use wry::WebViewExtMacOS as _;

use crate::{Error, WebViewEvent, WebViewSource, warn};

/// A wry webview, together with the [`WebViewEvent`]s it reported since the last
/// [`Self::take_events`].
///
/// The backend that owns it adds the placement and focus handling around it.
pub struct WryPage {
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
    /// # Errors
    /// If `source` is a URL that `NSURL` can't parse, or if wry fails to create the webview.
    pub fn new(
        ctx: &Context,
        source: &WebViewSource,
        parent: &impl HasWindowHandle,
    ) -> Result<Self, Error> {
        let events = Rc::new(RefCell::new(Vec::new()));
        let webview = wry::WebViewBuilder::new()
            // Otherwise wry activates the app when it creates the webview.
            .with_focused(false)
            .with_initialization_script(ipc::SCRIPT)
            .with_on_page_load_handler({
                let (ctx, events) = (ctx.clone(), Rc::clone(&events));
                move |event, url| {
                    events.borrow_mut().push(match event {
                        wry::PageLoadEvent::Started => WebViewEvent::Loading { url },
                        wry::PageLoadEvent::Finished => WebViewEvent::Loaded { url },
                    });
                    ctx.request_repaint();
                }
            })
            .build_as_child(parent)
            .map_err(Error::backend)?;
        // Register the native handler before the first document can run its scripts.
        let ipc = ipc::Ipc::new(ctx, webview.manager(), Rc::clone(&events));
        let page = Self {
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

/// Returns an error for a URL that `NSURL` can't parse, on which wry panics.
fn check_url(source: &WebViewSource) -> Result<(), Error> {
    if let WebViewSource::Url(url) = source
        && NSURL::URLWithString(&NSString::from_str(url)).is_none()
    {
        return Err(Error::backend(format!("Not a URL: {url:?}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// wry panics on a URL that `NSURL` can't parse, so `check_url` must reject it first.
    #[test]
    fn check_url_rejects_what_nsurl_cannot_parse() {
        let url = |url: &str| WebViewSource::Url(url.to_owned());
        assert!(check_url(&url("http://[::1")).is_err());
        assert!(check_url(&url("https://example.com/")).is_ok());
        assert!(check_url(&WebViewSource::Html("http://[::1".to_owned())).is_ok());
    }
}
