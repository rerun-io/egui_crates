//! Web pages as [egui](https://www.egui.rs/) widgets.
//!
//! ## Example
//! Create the page once, show it in `ui`, and place it in `output_hook`:
//!
//! ```no_run
//! use egui_webview::{WebViewInstance, WebViewSource};
//!
//! struct MyApp {
//!     webview: WebViewInstance,
//! }
//!
//! impl eframe::App for MyApp {
//!     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
//!         egui::CentralPanel::default().show(ui, |ui| {
//!             self.webview.ui(ui, ui.available_size());
//!         });
//!     }
//!
//!     fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
//!         self.webview.place(ctx, &output.platform_output.embedded_views);
//!     }
//! }
//!
//! fn main() -> eframe::Result {
//!     let _guard = egui_webview::init(); // Must come first, see `init`.
//!
//!     eframe::run_native(
//!         "My app",
//!         eframe::NativeOptions::default(),
//!         Box::new(|cc| {
//!             let source = WebViewSource::Url("https://www.egui.rs".parse()?);
//!             let webview = WebViewInstance::new(
//!                 &cc.egui_ctx,
//!                 egui::Id::unique("page"),
//!                 &source,
//!                 cc.embedded_views(),
//!             )?;
//!             Ok(Box::new(MyApp { webview }))
//!         }),
//!     )
//! }
//! ```
//!
//! ## How it works
//!
//! The app creates a [`WebViewInstance`] once, keeps it, calls [`WebViewInstance::ui`] in every
//! pass in which the page should be shown, and [`WebViewInstance::place`] from
//! [`eframe::App::output_hook`].
//!
//! A [`WebViewInstance`] shows its page through a [`Backend`], in one of two ways:
//!
//! - Off-screen rendering (CEF, except on macOS without the `cef` feature): the page renders into a texture that egui
//!   paints like any other image, and the widget forwards egui's input to the page.
//! - An embedded view (`WKWebView` on macOS, an `<iframe>` on the web): the page lies below egui's
//!   surface. Its rect is an [`egui::EmbeddedView`], which clears the rect to
//!   transparent so the page shows through. After every pass of the root viewport,
//!   [`WebViewInstance::place`] moves the page to where egui put that rect, which egui reports
//!   as an [`egui::EmbeddedViewOutput`]. egui layers above the page (windows, popups, …) paint
//!   over it, and the output tells the backend where egui takes the pointer input.
//!
//! Tab and Shift-Tab move the keyboard focus between egui widgets and the page's elements; see
//! [`WebViewInstance::ui`].
//!
//! A [`WebViewInstance`] isn't `Send`. If the code that shows the page can't keep one, use
//! [`WebView`]. It shows a page with one call, and the pages live in a [`WebViews`] that the app
//! owns.
//!
//! Call [`init`] first in `main`, and keep the [`ShutdownGuard`] it returns until
//! `eframe::run_native` has returned.
//!
//! ## CEF
//!
//! - Process-wide options, such as a persistent profile directory, go in `CefInitOptions`, passed
//!   to `cef_init_with`. By default, nothing persists.
//! - CEF's helper processes run without Chromium's sandbox, unless `CefInitOptions::sandbox` is
//!   `true`, which needs the `cef-sandbox` feature. Without the sandbox, only show pages you
//!   trust.
//! - A page that asks for a new window sends [`WebViewEvent::NewWindowRequested`] and opens
//!   nothing.
//! - While a CEF webview exists, egui repaints at least every 100 ms, because CEF's message loop
//!   runs in egui's passes.
//!
//! ## Feature flags
#![cfg_attr(feature = "document-features", doc = document_features::document_features!())]

mod backend;
#[cfg(cef_backend)]
mod chromium;
mod instance;
#[cfg(wry_backend)]
mod macos;
mod mailbox;
#[cfg(target_arch = "wasm32")]
mod web;
mod webviews;
#[cfg(wry_backend)]
mod wry_page;

pub use backend::{Backend, EmbeddedWidget, PageWidget};
#[cfg(cef_backend)]
pub use chromium::{
    CefInitOptions, CefOptions, CefProcess, init as cef_init, init_with as cef_init_with,
    run_subprocess as run_cef_subprocess,
};
pub use instance::{WebViewInstance, WebViewResponse};
pub use webviews::{WebView, WebViews};

pub use url::{self, Url};

/// Initializes `egui_webview`. Call this first in `main`, whatever the platform and whichever
/// features are enabled, and keep the guard until `eframe::run_native` has returned:
///
/// ```no_run
/// let _guard = egui_webview::init();
/// // … run the app
/// ```
///
/// Currently, it only does something where CEF is built, and then it calls
/// `egui_webview::cef_init`: CEF starts its helper processes (renderer, GPU, …) from the app's
/// own executable, and in such a process this function runs the helper and then exits the
/// process instead of returning. Elsewhere, such as on the web, it does nothing.
pub fn init() -> ShutdownGuard {
    cfg_select! {
        cef_backend => { cef_init() }
        _ => { ShutdownGuard { _runtime: None } }
    }
}

/// Keeps the webview runtime alive on the thread that initialized it.
///
/// Keep the guard until `eframe::run_native` returns. Repeated calls to [`init`] on that
/// thread share the runtime. Each CEF webview also holds it until dropped. After the last
/// owner is dropped, CEF shuts down once its browsers finish closing.
#[must_use = "keep the guard while using webviews"]
#[derive(Debug)]
pub struct ShutdownGuard {
    _runtime: Option<std::rc::Rc<Shutdown>>,
}

#[derive(Debug)]
struct Shutdown;

impl Drop for Shutdown {
    fn drop(&mut self) {
        #[cfg(cef_backend)]
        chromium::shutdown();
    }
}

/// What should the webview load?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewSource {
    /// An HTML document, given as its source text.
    Html(String),

    /// The URL of a page.
    Url(Url),
}

impl WebViewSource {
    /// Returns the HTML source if this is [`Self::Html`].
    pub fn html(&self) -> Option<&str> {
        match self {
            Self::Html(html) => Some(html),
            Self::Url(_) => None,
        }
    }
}

impl From<Url> for WebViewSource {
    fn from(url: Url) -> Self {
        Self::Url(url)
    }
}

/// Events returned by the webview.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebViewEvent {
    /// A page started loading.
    Loading {
        /// The URL of the page. For a page given as [`WebViewSource::Html`], this is
        /// `about:srcdoc` on the web, and a `data:` URL elsewhere.
        url: Url,
    },

    /// A page finished loading.
    Loaded {
        /// The URL of the page, as in [`Self::Loading`].
        url: Url,
    },

    /// The page asked for a new window, with `window.open` or a link with `target="_blank"`.
    ///
    /// The webview opens no window and doesn't navigate. The app decides what to do, for example
    /// load the URL with [`WebViewInstance::load_url`] or open it in the system browser.
    ///
    /// Only the CEF backend sends this. On the web, the browser handles new windows itself.
    NewWindowRequested {
        /// The URL that the page wants to open.
        url: Url,

        /// Whether the page asked in response to a user action, such as a click. A page that
        /// asks without one is often showing an unwanted popup.
        user_gesture: bool,
    },

    /// A string that the page sent with `window.ipc.postMessage(text)`.
    ///
    /// On the web, a page loaded from a URL has no `window.ipc`. It sends messages with
    /// `window.parent.postMessage(text, "*")` instead, and only string messages are reported.
    Message(String),
}

/// Logs a warning for a failed call whose error isn't returned to the caller.
pub(crate) fn warn<E: std::fmt::Display>(result: Result<(), E>) {
    if let Err(err) = result {
        log::warn!("egui_webview: {err}");
    }
}

/// Parses a URL that a backend's engine reported for a [`WebViewEvent`].
///
/// Engines report valid URLs, so this only logs a warning for one that doesn't parse.
#[cfg(any(cef_backend, platform_backend))]
pub(crate) fn reported_url(url: &str) -> Option<Url> {
    Url::parse(url)
        .inspect_err(|err| {
            log::warn!("egui_webview: the engine reported an invalid URL {url:?}: {err}");
        })
        .ok()
}

/// An error from creating or using a [`WebViewInstance`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// No backend supports this platform, or the window handle of [`eframe::EmbeddedViews`] is
    /// not one the platform's backend can use.
    Unsupported,

    /// The backend reported an error.
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

impl Error {
    /// Wraps `err` in [`Self::Backend`].
    pub fn backend(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Backend(err.into())
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => write!(f, "No webview backend is available on this platform"),
            Self::Backend(err) => write!(f, "Webview failed: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported => None,
            Self::Backend(err) => Some(err.as_ref()),
        }
    }
}
