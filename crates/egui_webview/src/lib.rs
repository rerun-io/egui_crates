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
//! - A native view (an `<iframe>` on the web): the page lies below egui's surface. Its rect is an [`egui::NativeView`], which clears the rect to
//!   transparent so the page shows through. After every pass of the root viewport,
//!   [`WebViewInstance::place`] moves the page to its [`Placement`]. egui layers above the page
//!   (windows, popups, …) paint over it, and the [`Placement`] tells the backend where egui takes
//!   the pointer input.
//!
//! Tab and Shift-Tab move the keyboard focus between egui widgets and the page's elements; see
//! [`WebViewInstance::ui`].

mod backend;
#[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
mod chromium;
mod placement;
#[cfg(target_arch = "wasm32")]
mod web;
mod webview;

pub use backend::Backend;
#[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
pub use chromium::run_subprocess as run_cef_subprocess;
pub use placement::Placement;
pub use webview::{WebViewInstance, WebViewResponse};

/// What should the webview load?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewSource {
    /// An HTML document, given as its source text.
    Html(String),

    /// The URL of a page.
    Url(String),
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

/// Events returned by the webview.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WebViewEvent {
    /// A page started loading.
    Loading {
        /// The URL of the page. For a page given as [`WebViewSource::Html`], this is
        /// `about:srcdoc`, `about:blank` or a `data:` URL, depending on the backend.
        url: String,
    },

    /// A page finished loading.
    Loaded {
        /// The URL of the page, as in [`Self::Loading`].
        url: String,
    },

    /// A string that the page sent with `window.ipc.postMessage(text)`.
    ///
    /// On the web, a page loaded from a URL has no `window.ipc`. It sends messages with
    /// `window.parent.postMessage(text, "*")` instead, and only string messages are reported.
    Message(String),
}

/// An error from creating or using a [`WebViewInstance`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// No backend supports this platform, or the window handle of [`eframe::NativeViews`] is
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
