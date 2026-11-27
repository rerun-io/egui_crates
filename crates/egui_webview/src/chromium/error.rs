//! Why CEF can't start, or can't do what a page asks of it.

use std::path::PathBuf;
use std::sync::Arc;

/// An error from CEF, or from getting CEF to run.
///
/// It is `Clone` because the first failure to start CEF is kept, and every later page reports it.
#[derive(Clone, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CefError {
    /// A webview was created on another thread than the one that created the
    /// [`crate::Runtime`].
    #[error("CEF: create webviews on the thread that created the `egui_webview::Runtime`")]
    WrongThread,

    /// The app's `main` hasn't shown that it handles being a CEF helper process.
    #[error("CEF: create the `egui_webview::Runtime` with `Runtime::new` first in `main`")]
    NoRuntimeInMain,

    /// CEF has shut down, and can't start again in this process.
    #[error(
        "CEF has shut down, because the `egui_webview::Runtime` was dropped. Keep it until \
         `eframe::run_native` returns"
    )]
    ShutDown,

    /// The path of this process's executable is unknown.
    #[error("CEF: can't find the path of the executable: {0}")]
    CurrentExe(Arc<std::io::Error>),

    /// The executable has no parent directory to look for CEF in.
    #[cfg(target_os = "macos")]
    #[error("CEF: the executable `{}` has no parent directory", exe.display())]
    NoExeDir {
        /// The path of the executable.
        exe: PathBuf,
    },

    /// The CEF framework is in none of the places it can be.
    #[cfg(target_os = "macos")]
    #[error(
        "The CEF framework is neither in the app bundle ({bundled:?}) nor in the download \
         directory of the `cef` crate. Bundle the framework with the app, or set `CEF_PATH` to \
         the directory of a CEF download"
    )]
    FrameworkNotFound {
        /// Where the framework would be in an app bundle.
        bundled: Vec<PathBuf>,
    },

    /// The CEF framework is there, but can't be loaded.
    #[cfg(target_os = "macos")]
    #[error("Failed to load the CEF framework from `{}`", path.display())]
    LoadFramework {
        /// The path of the framework's library.
        path: PathBuf,
    },

    /// [`crate::CefInitOptions::sandbox`] is set without the `cef-sandbox` feature.
    #[error("CEF: `CefInitOptions::sandbox` needs the `cef-sandbox` feature")]
    SandboxNeedsFeature,

    /// The CEF sandbox isn't supported on Windows.
    #[cfg(windows)]
    #[error("The CEF sandbox isn't supported on Windows")]
    SandboxOnWindows,

    /// On macOS, the CEF sandbox only works in an app bundle with CEF's helper apps.
    #[cfg(target_os = "macos")]
    #[error("The CEF sandbox needs an app bundle with CEF's helper apps on macOS")]
    SandboxNeedsAppBundle,

    /// On macOS, a sandboxed helper process must run from a helper app in the app bundle.
    #[cfg(all(target_os = "macos", feature = "cef-sandbox"))]
    #[error("The CEF sandbox needs the helper to run from a helper app in an app bundle")]
    SandboxedHelperOutsideBundle,

    /// [`crate::CefInitOptions::cache_dir`] can't be made into an absolute path.
    #[error("CEF: the cache directory `{}` isn't a usable path: {err}", dir.display())]
    CacheDir {
        /// The directory that the app asked for.
        dir: PathBuf,

        /// Why it can't be used.
        err: Arc<std::io::Error>,
    },

    /// The temporary directory for an in-memory profile can't be created.
    #[error("CEF: failed to create a temporary cache directory: {0}")]
    TempCacheDir(Arc<std::io::Error>),

    /// `cef::initialize` failed.
    #[error(
        "CEF failed to initialize, with exit code {exit_code}{}",
        cache_dir_hint(cache_dir.as_deref())
    )]
    Initialize {
        /// CEF's exit code, one of `cef_resultcode_t`.
        exit_code: i32,

        /// The cache directory that the app asked for, which only one process can use.
        cache_dir: Option<PathBuf>,
    },

    /// CEF created no browser for a new page.
    #[error("CEF failed to create a browser")]
    CreateBrowser,

    /// The browser has no main frame, which is the case once it is closed.
    #[error("The CEF browser has no main frame")]
    NoMainFrame,
}

/// What to check when CEF fails to initialize with a cache directory.
fn cache_dir_hint(cache_dir: Option<&std::path::Path>) -> String {
    cache_dir.map_or_else(String::new, |dir| {
        format!(
            ". Only one running process can use the cache directory `{}`, so check that no \
             other instance of the app is running",
            dir.display()
        )
    })
}

impl From<CefError> for crate::Error {
    fn from(err: CefError) -> Self {
        Self::backend(err)
    }
}
