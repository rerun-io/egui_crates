//! [`Runtime`], which keeps the webview engine alive.

#[cfg(not(target_arch = "wasm32"))]
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::atomic::{AtomicBool, Ordering},
};

/// Whether a [`Runtime`] is alive in this process.
#[cfg(not(target_arch = "wasm32"))]
static ALIVE: AtomicBool = AtomicBool::new(false);

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// The engine of this thread, while the [`Runtime`] or a webview keeps it alive.
    static ENGINE: RefCell<Weak<Owner>> = const { RefCell::new(Weak::new()) };
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
/// Creating a [`Runtime`] is cheap. It starts no engine, process, or thread. CEF starts when the
/// first CEF webview is created. So don't wait with the [`Runtime`] until a webview is shown:
/// CEF's helper processes run the app's `main`, and must reach [`Runtime::new`] before the app
/// does anything else.
///
/// Natively, creating a webview without a running [`Runtime`] fails with
/// [`crate::Error::NoRuntime`]. On the web, the browser is the engine, and a [`Runtime`] does
/// nothing.
///
/// Natively, only one [`Runtime`] may be alive at a time. Creating a second one panics.
///
/// Each CEF webview keeps the engine alive until dropped. After the [`Runtime`] and the last
/// webview are dropped, CEF shuts down once its browsers finish closing, and can't start again in
/// this process.
#[must_use = "keep the runtime until `eframe::run_native` returns"]
#[derive(Debug)]
#[cfg_attr(target_arch = "wasm32", expect(missing_copy_implementations))] // Not `Copy` natively.
pub struct Runtime {
    #[cfg(not(target_arch = "wasm32"))]
    _engine: Engine,
}

/// Keeps the webview engine alive.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(crate) struct Engine {
    _owner: Rc<Owner>,
}

impl Runtime {
    /// Creates the webview runtime. Call this at the start of `main`, right after initializing
    /// logging.
    ///
    /// This is cheap: it doesn't start CEF or any of its processes. That happens when the first
    /// CEF webview is created.
    ///
    /// With the `cef` feature, this is `Self::with_cef_options` with the default options. CEF
    /// launches its helper processes (renderer, GPU, …) from the app's own executable, so `main`
    /// also runs in each of them. In a helper process, this call hands the process over to CEF,
    /// and exits the process when the helper is done, instead of returning.
    ///
    /// # Panics
    /// Natively, if another [`Runtime`] is alive.
    #[expect(clippy::new_without_default)] // Starting a runtime is more than a default value.
    pub fn new() -> Self {
        cfg_select! {
            target_arch = "wasm32" => { Self {} }
            feature = "cef" => { Self::with_cef_options(crate::CefInitOptions::default()) }
            _ => { Self::start() }
        }
    }

    /// Like [`Self::new`], with `options` for when the first webview starts CEF.
    ///
    /// The options apply to the whole process:
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

    /// Starts the runtime, on the engine of this thread if webviews still keep one alive.
    ///
    /// # Panics
    /// If another [`Runtime`] is alive.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start() -> Self {
        assert!(
            !ALIVE.swap(true, Ordering::SeqCst),
            "egui_webview: only one `egui_webview::Runtime` may be alive at a time"
        );
        let owner = ENGINE.with_borrow_mut(|engine| {
            engine.upgrade().unwrap_or_else(|| {
                let owner = Rc::new(Owner);
                *engine = Rc::downgrade(&owner);
                owner
            })
        });
        Self {
            _engine: Engine { _owner: owner },
        }
    }

    /// The engine of the [`Runtime`], for a webview to keep alive.
    ///
    /// # Errors
    /// [`crate::Error::NoRuntime`] if no [`Runtime`] on this thread is alive.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn engine() -> Result<Engine, crate::Error> {
        ALIVE
            .load(Ordering::SeqCst)
            .then(|| ENGINE.with_borrow(Weak::upgrade))
            .flatten()
            .map(|owner| Engine { _owner: owner })
            .ok_or(crate::Error::NoRuntime)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Runtime {
    fn drop(&mut self) {
        ALIVE.store(false, Ordering::SeqCst);
    }
}

/// Shuts the engine down when the [`Runtime`] and the last webview are dropped.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct Owner;

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Owner {
    fn drop(&mut self) {
        #[cfg(feature = "cef")]
        crate::chromium::shutdown();
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::Error;

    // One test, because the runtime is process-wide.
    #[test]
    fn one_runtime_at_a_time() {
        assert!(matches!(Runtime::engine(), Err(Error::NoRuntime)));

        let runtime = Runtime::start();
        let engine = Runtime::engine();
        assert!(engine.is_ok());
        // Also on another thread.
        let second = std::thread::Builder::new()
            .name("second_runtime".to_owned())
            .spawn(|| drop(Runtime::start()))
            .map(std::thread::JoinHandle::join);
        assert!(matches!(second, Ok(Err(_))));

        drop(runtime);
        assert!(matches!(Runtime::engine(), Err(Error::NoRuntime)));

        // A webview's engine outlives the runtime, and a new runtime takes it over.
        drop(Runtime::start());
        drop(engine);
        assert!(matches!(Runtime::engine(), Err(Error::NoRuntime)));
    }
}
