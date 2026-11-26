//! [`Runtime`], which keeps the webview engine alive.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::Error;

thread_local! {
    /// The runtime of this thread, while some [`Runtime`] keeps it alive.
    static CURRENT: RefCell<Weak<Owner>> = const { RefCell::new(Weak::new()) };
}

/// Keeps the webview engine alive on the thread that created it.
///
/// Create it at the start of `main`, right after initializing logging, whatever the platform and
/// whichever features are enabled. Keep it until `eframe::run_native` has returned:
///
/// ```no_run
/// // Initialize logging first, so that CEF's helper processes log too.
/// let _runtime = egui_webview::Runtime::new();
/// // … run the app
/// ```
///
/// Without a running [`Runtime`], creating a webview fails with [`Error::NoRuntime`].
///
/// Clones, and later calls to [`Self::new`] on the same thread, share the runtime. Each CEF
/// webview also keeps it until dropped. After the last owner is dropped, CEF shuts down once its
/// browsers finish closing, and can't start again in this process.
#[must_use = "keep the runtime until `eframe::run_native` returns"]
#[derive(Clone, Debug)]
pub struct Runtime {
    _owner: Rc<Owner>,
}

impl Runtime {
    /// Starts the webview runtime. Call this at the start of `main`, right after initializing
    /// logging.
    ///
    /// Currently, it only does something natively with the `cef` feature, where it is
    /// `Self::with_cef_options` with the default options: CEF starts its helper processes
    /// (renderer, GPU, …) from the app's own executable, and in such a process this runs the
    /// helper and then exits the process instead of returning.
    #[expect(clippy::new_without_default)] // Starting a runtime is more than a default value.
    pub fn new() -> Self {
        cfg_select! {
            all(feature = "cef", not(target_arch = "wasm32")) => {
                Self::with_cef_options(crate::CefInitOptions::default())
            }
            _ => { Self::start() }
        }
    }

    /// Like [`Self::new`], with `options` for when the first webview starts CEF.
    ///
    /// The first call in the process sets the process-wide options:
    ///
    /// ```no_run
    /// let options = egui_webview::CefInitOptions {
    ///     cache_dir: Some("my_app_profile".into()),
    ///     ..Default::default()
    /// };
    /// let _runtime = egui_webview::Runtime::with_cef_options(options);
    /// // … run the app
    /// ```
    ///
    /// To handle the exit code of a helper process yourself, call
    /// [`crate::run_cef_subprocess`] instead.
    #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
    pub fn with_cef_options(options: crate::CefInitOptions) -> Self {
        crate::chromium::init(options)
    }

    /// Shares the running runtime of this thread, or starts a new one.
    pub(crate) fn start() -> Self {
        CURRENT.with_borrow_mut(|current| {
            let owner = current.upgrade().unwrap_or_else(|| {
                let owner = Rc::new(Owner);
                *current = Rc::downgrade(&owner);
                owner
            });
            Self { _owner: owner }
        })
    }

    /// The running runtime of this thread.
    ///
    /// # Errors
    /// [`Error::NoRuntime`] if no [`Runtime`] on this thread is alive.
    pub(crate) fn current() -> Result<Self, Error> {
        CURRENT
            .with_borrow(Weak::upgrade)
            .map(|owner| Self { _owner: owner })
            .ok_or(Error::NoRuntime)
    }
}

/// Shuts the engine down when the last [`Runtime`] is dropped.
#[derive(Debug)]
struct Owner;

impl Drop for Owner {
    fn drop(&mut self) {
        #[cfg(all(feature = "cef", not(target_arch = "wasm32")))]
        crate::chromium::shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_needs_a_live_runtime() {
        assert!(matches!(Runtime::current(), Err(Error::NoRuntime)));

        let runtime = Runtime::start();
        let shared = Runtime::start();
        drop(runtime);
        assert!(Runtime::current().is_ok());

        drop(shared);
        assert!(matches!(Runtime::current(), Err(Error::NoRuntime)));
    }
}
