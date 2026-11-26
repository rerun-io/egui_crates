//! Web pages as [egui](https://www.egui.rs/) widgets.
//!
//! The app creates a [`WebViewInstance`] once, keeps it, calls [`WebViewInstance::ui`] in every
//! pass in which the page should be shown, and [`WebViewInstance::place`] from
//! [`eframe::App::output_hook`].
//!
//! A [`WebViewInstance`] shows its page through a [`Backend`], in one of two ways:
//!
//! - Off-screen rendering (CEF, with the `cef` feature): the page renders into a texture that egui
//!   paints like any other image, and the widget forwards egui's input to the page.
//! - An embedded view (an `<iframe>` on the web): the page lies below egui's surface. Its rect is an [`egui::EmbeddedView`], which clears the rect to
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
//! Create a [`Runtime`] first in `main`, and keep it until `eframe::run_native` has returned.
//!
//! ## CEF
//!
//! - Process-wide options, such as a persistent profile directory, go in `CefInitOptions`, passed
//!   to `Runtime::with_cef_options`. By default, nothing persists.
//! - CEF's helper processes run without Chromium's sandbox, unless `CefInitOptions::sandbox` is
//!   `true`, which needs the `cef-sandbox` feature. Without the sandbox, only show pages you
//!   trust.
//! - A page that asks for a new window sends [`WebViewEvent::NewWindowRequested`] and opens
//!   nothing.
//! - While a CEF webview exists, egui repaints at least every 100 ms, because CEF's message loop
//!   runs in egui's passes.

mod backend;
#[cfg(cef_backend)]
mod chromium;
mod instance;
mod mailbox;
mod runtime;
#[cfg(platform_backend)]
mod web;
mod webviews;

pub use backend::{Backend, EmbeddedWidget, PageWidget};
#[cfg(all(feature = "cef-download", cef_backend))]
pub use chromium::install as cef_install;
#[cfg(cef_backend)]
pub use chromium::{
    CefInitOptions, CefOptions, CefProcess, CefStatus, run_subprocess as run_cef_subprocess,
    status as cef_status,
};
pub use instance::{WebViewInstance, WebViewResponse};
pub use runtime::Runtime;
pub use webviews::{WebView, WebViews};

pub use url::{self, Url};

/// Whether this build has a backend that can show pages.
///
/// That is the `<iframe>` backend on the web, and CEF on Linux, macOS, and Windows with the
/// `cef` feature.
/// Without a backend, [`WebViewInstance::new`] returns an error.
pub const fn is_supported() -> bool {
    cfg!(any(cef_backend, platform_backend))
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
        /// `about:srcdoc`, `about:blank` or a `data:` URL, depending on the backend.
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
    /// No [`Runtime`] is running on this thread. Never happens on the web.
    NoRuntime,

    /// No backend supports this platform, or the window handle of [`eframe::EmbeddedViews`] is
    /// not one the platform's backend can use.
    Unsupported,

    /// CEF's files aren't on this machine. See `CefInitOptions::cef_dir` for where they go,
    /// and `cef_status`.
    ///
    /// Only the CEF backend returns this.
    CefNotInstalled,

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
            Self::NoRuntime => write!(
                f,
                "No running egui_webview::Runtime. Create one with `egui_webview::Runtime::new()` \
                 first in `main`, and keep it until `eframe::run_native` returns"
            ),
            Self::Unsupported => write!(f, "No webview backend is available on this platform"),
            Self::CefNotInstalled => write!(f, "CEF (Chromium) isn't installed"),
            Self::Backend(err) => write!(f, "Webview failed: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoRuntime | Self::Unsupported | Self::CefNotInstalled => None,
            Self::Backend(err) => Some(err.as_ref()),
        }
    }
}
