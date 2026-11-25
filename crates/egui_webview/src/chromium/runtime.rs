//! CEF's process-wide initialization, the entry point of its helper processes, and the external
//! message pump that egui's passes drive.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock, Weak};
use std::time::Duration;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    App, Browser, BrowserProcessHandler, CefString, ImplApp, ImplBrowserProcessHandler,
    ImplCommandLine as _, ImplFrame as _, ImplRenderProcessHandler, RenderProcessHandler, WrapApp,
    WrapBrowserProcessHandler, WrapRenderProcessHandler, sys, wrap_app,
    wrap_browser_process_handler, wrap_render_process_handler,
};
use cef::{
    ImplListValue as _, ImplProcessMessage as _, ImplV8Context as _, ImplV8Handler,
    ImplV8Value as _, V8Handler, V8Value, WrapV8Handler, wrap_v8_handler,
};
use egui::Ui;

use super::context::Context;
use crate::Error;

/// The name of the process message that brings the text of a page's
/// `window.ipc.postMessage(text)` from the render process to the browser process.
pub const IPC_MESSAGE: &str = "egui_webview_ipc";

/// The longest time between two calls to `cef::do_message_loop_work` while a webview exists.
///
/// With an external message pump, CEF schedules its work through
/// `on_schedule_message_pump_work`, but it can also have work pending that it didn't schedule.
pub const MAX_PUMP_DELAY: Duration = Duration::from_millis(100);

/// The result of initializing CEF, which can happen only once per process.
static INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Whether [`run_subprocess`] has run. Without it, CEF's helper processes would run the app.
static RAN_SUBPROCESS_CHECK: AtomicBool = AtomicBool::new(false);

/// The live pages, whose egui contexts [`wake`] repaints when CEF schedules work.
///
/// Weak, so that an egui context isn't kept alive after its last page is gone.
#[expect(clippy::disallowed_types)] // egui's `Mutex` has no `const` constructor for a `static`.
static PAGES: std::sync::Mutex<Vec<Weak<Context>>> = std::sync::Mutex::new(Vec::new());

/// Runs this process as a CEF helper process and exits, if Chromium started it as one. Otherwise
/// it returns right away.
///
/// CEF starts its helper processes (renderer, GPU, …) from the app's own executable, so this
/// must run first in `main`. [`crate::init`] does that when the `cef` feature is enabled, so
/// apps should call that instead:
///
/// ```no_run
/// egui_webview::init();
/// // … start the app
/// ```
///
/// To handle the helper's exit code yourself, call [`crate::run_cef_subprocess`] instead.
pub fn init() {
    if let Some(code) = run_subprocess() {
        #[expect(clippy::exit)] // A helper process has nothing of the app to unwind.
        std::process::exit(code);
    }
}

/// Runs this process as a CEF helper process, if Chromium started it as one, and returns its exit
/// code.
///
/// This is [`crate::cef_init`] without the exit. Call it at the start of `main`, and exit with
/// the code it returns:
///
/// ```no_run
/// if let Some(code) = egui_webview::run_cef_subprocess() {
///     std::process::exit(code);
/// }
/// ```
pub fn run_subprocess() -> Option<i32> {
    RAN_SUBPROCESS_CHECK.store(true, Ordering::Relaxed);
    // A process that can't load the library is no helper process, because the process that
    // starts the helpers has loaded the same library. `initialize` reports the error when the
    // app creates its first webview.
    load_library().ok()?;
    let args = cef::args::Args::new();
    let mut app = EguiApp::new();
    let code = cef::execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    // CEF decides what kind of process this is. In the browser process, which is the app
    // itself, it returns -1 right away.
    (0 <= code).then_some(code)
}

/// Loads the CEF library, and selects the CEF API version that these bindings were generated
/// for. Both come before every other CEF call, and happen once per process.
fn load_library() -> Result<(), String> {
    static LOADED: LazyLock<Result<(), String>> = LazyLock::new(|| {
        // On other platforms, the CEF library is linked at build time.
        #[cfg(target_os = "macos")]
        load_framework()?;
        _ = cef::api_hash(sys::CEF_API_VERSION_LAST, 0);
        Ok(())
    });
    LOADED.clone()
}

/// Loads the CEF framework on macOS, from the app bundle if there is one, and otherwise from where
/// the `cef` crate downloaded it at build time.
#[cfg(target_os = "macos")]
fn load_framework() -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt as _;

    const FRAMEWORK: &str = "Chromium Embedded Framework.framework/Chromium Embedded Framework";
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let exe_dir = exe.parent().ok_or("no executable directory")?;
    // The framework is in `Contents/Frameworks`. That is `../Frameworks` from the main
    // executable in `Contents/MacOS`, and `../../..` from a helper's executable in
    // `Contents/Frameworks/<App> Helper.app/Contents/MacOS`.
    let bundled = [exe_dir.join("../Frameworks"), exe_dir.join("../../..")]
        .into_iter()
        .map(|dir| dir.join(FRAMEWORK))
        .find(|path| path.exists());
    let path = bundled
        .or_else(|| sys::get_cef_dir().map(|dir| dir.join(FRAMEWORK)))
        .ok_or("CEF framework not found")?;
    let path =
        std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(|err| err.to_string())?;
    // SAFETY: `path` is a NUL-terminated C string that lives until after `cef::load_library`
    // returns.
    #[expect(unsafe_code)]
    let name = unsafe { &*path.as_ptr() };
    if cef::load_library(Some(name)) == 1 {
        Ok(())
    } else {
        Err(format!("Failed to load the CEF framework from {path:?}"))
    }
}

/// Initializes CEF once per process, and has `ctx` run CEF's message loop work at the start of
/// each of its passes.
///
/// # Errors
/// If [`run_subprocess`] hasn't run, if the CEF library can't be loaded, or if CEF fails to
/// initialize. The first result is kept, so later calls fail in the same way.
pub fn initialize(ctx: &egui::Context) -> Result<(), Error> {
    let result = INIT.get_or_init(|| {
        // CEF starts its helper processes from this executable. Refuse to start them before the
        // app has shown that its `main` handles being one.
        if !RAN_SUBPROCESS_CHECK.load(Ordering::Relaxed) {
            return Err("call `egui_webview::init` first in `main`".to_owned());
        }
        load_library()?;

        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let mut settings = cef::Settings {
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            no_sandbox: 1,
            browser_subprocess_path: exe.to_string_lossy().as_ref().into(),
            root_cache_path: cache_dir().to_string_lossy().as_ref().into(),
            log_severity: sys::cef_log_severity_t::LOGSEVERITY_WARNING.into(),
            ..Default::default()
        };
        resource_paths(&exe, &mut settings);

        // CEF keeps pointers into the arguments for the life of the process.
        let args = Box::leak(Box::new(cef::args::Args::new()));
        let mut app = EguiApp::new();
        if cef::initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        ) == 1
        {
            Ok(())
        } else {
            Err("CEF failed to initialize".to_owned())
        }
    });
    result
        .clone()
        .map_err(|err| Error::backend(format!("CEF: {err}")))?;

    // egui ignores a second plugin of the same type.
    ctx.add_plugin(CefPlugin);
    Ok(())
}

/// Has [`wake`] repaint the egui context of `page` while the page lives.
pub fn register_page(page: &Arc<Context>) {
    let mut pages = lock_pages();
    pages.retain(|page| 0 < page.strong_count());
    pages.push(Arc::downgrade(page));
}

fn lock_pages() -> std::sync::MutexGuard<'static, Vec<Weak<Context>>> {
    PAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// CEF's root cache directory, which is unique to this process so that two running instances of
/// an app don't share one.
///
/// The directory is never removed, because CEF can write to it until the process exits.
#[expect(clippy::disallowed_methods)] // A `tempdir` would be deleted while CEF still uses it.
fn cache_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("egui_webview_{}", std::process::id()))
}

/// Points CEF at the resources the `cef` crate downloaded at build time, unless they are next to
/// the executable, as in a packaged app.
fn resource_paths(exe: &std::path::Path, settings: &mut cef::Settings) {
    let Some(cef_dir) = sys::get_cef_dir() else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        let in_bundle = exe
            .parent()
            .is_some_and(|dir| dir.join("../Frameworks").exists());
        if !in_bundle {
            settings.framework_dir_path = cef_dir
                .join("Chromium Embedded Framework.framework")
                .to_string_lossy()
                .as_ref()
                .into();
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let next_to_exe = exe
            .parent()
            .is_some_and(|dir| dir.join("icudtl.dat").exists());
        if !next_to_exe {
            settings.resources_dir_path = cef_dir.to_string_lossy().as_ref().into();
            settings.locales_dir_path = cef_dir.join("locales").to_string_lossy().as_ref().into();
        }
    }
}

/// An egui plugin that runs CEF's message loop work at the start of every pass.
struct CefPlugin;

impl egui::Plugin for CefPlugin {
    fn debug_name(&self) -> &'static str {
        "egui_webview::cef"
    }

    fn on_begin_pass(&mut self, _ui: &mut Ui) {
        cef::do_message_loop_work();
    }
}

/// Requests a repaint of the egui context of every page in [`PAGES`] after `delay`, so that its
/// next pass runs CEF's message loop work. Forgets the pages that are gone.
fn wake(delay: Duration) {
    lock_pages().retain(|page| {
        let page = page.upgrade();
        if let Some(page) = &page {
            page.egui_ctx.request_repaint_after(delay);
        }
        page.is_some()
    });
}

wrap_app! {
    struct EguiApp {}

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut cef::CommandLine>,
        ) {
            let Some(command_line) = command_line else {
                return;
            };
            let is_browser = process_type.is_none_or(|t| t.to_string().is_empty());
            if !is_browser {
                return;
            }
            // Use a mock keychain, so that Chromium doesn't show a macOS keychain access prompt.
            command_line.append_switch(Some(&"use-mock-keychain".into()));
            // Pick the Ozone platform of the session, unless the command line picks one.
            #[cfg(target_os = "linux")]
            if command_line.has_switch(Some(&"ozone-platform".into())) == 0 {
                let platform = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                    "wayland"
                } else {
                    "x11"
                };
                command_line.append_switch_with_value(
                    Some(&"ozone-platform".into()),
                    Some(&platform.into()),
                );
            }
            // On Wayland with fractional scaling, Chromium ignores the device scale factor from
            // `screen_info` and renders a windowless browser at 1 pixel per point. A windowless
            // browser has no Wayland surface to scale, so it loses nothing without the feature.
            #[cfg(target_os = "linux")]
            {
                let name = CefString::from("disable-features");
                let mut features = CefString::from(&command_line.switch_value(Some(&name)))
                    .to_string();
                if !features.is_empty() {
                    features.push(',');
                }
                features.push_str("WaylandFractionalScaleV1");
                command_line.append_switch_with_value(Some(&name), Some(&features.as_str().into()));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(EguiBrowserProcessHandler::new())
        }

        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(EguiRenderProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    struct EguiBrowserProcessHandler {}

    impl BrowserProcessHandler {
        /// Called by CEF, on any thread, to schedule message loop work after `delay_ms`.
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            wake(Duration::from_millis(delay_ms.max(0).unsigned_abs()));
        }
    }
}

wrap_render_process_handler! {
    struct EguiRenderProcessHandler {}

    impl RenderProcessHandler {
        /// Defines `window.ipc.postMessage(text)` in the main frame's JavaScript context, before
        /// the page's scripts run. Subframes don't get it, because they can come from any origin.
        fn on_context_created(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            context: Option<&mut cef::V8Context>,
        ) {
            if frame.is_none_or(|frame| frame.is_main() == 0) {
                return;
            }
            let Some(global) = context.and_then(|context| context.global()) else {
                return;
            };
            let mut handler = EguiIpcHandler::new();
            let post_message =
                cef::v8_value_create_function(Some(&"postMessage".into()), Some(&mut handler));
            let (Some(mut post_message), Some(mut ipc)) =
                (post_message, cef::v8_value_create_object(None, None))
            else {
                return;
            };
            // Read-only and non-deletable, so that the page can't replace them.
            let fixed = sys::cef_v8_propertyattribute_t(
                sys::cef_v8_propertyattribute_t::V8_PROPERTY_ATTRIBUTE_READONLY.0
                    | sys::cef_v8_propertyattribute_t::V8_PROPERTY_ATTRIBUTE_DONTDELETE.0,
            );
            ipc.set_value_bykey(Some(&"postMessage".into()), Some(&mut post_message), fixed.into());
            global.set_value_bykey(Some(&"ipc".into()), Some(&mut ipc), fixed.into());
        }
    }
}

wrap_v8_handler! {
    struct EguiIpcHandler {}

    impl V8Handler {
        /// Implements `window.ipc.postMessage(text)` by sending `text` in an [`IPC_MESSAGE`] from
        /// the calling frame to the browser process. Throws a JavaScript exception if `text` isn't
        /// a string.
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            _retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let text = arguments
                .and_then(|arguments| arguments.first())
                .and_then(Option::as_ref)
                .filter(|value| value.is_string() == 1)
                .map(|value| CefString::from(&value.string_value()));
            let Some(text) = text else {
                if let Some(exception) = exception {
                    // `try_set` writes into CEF's string. Assigning would only replace the
                    // local wrapper.
                    exception.try_set("ipc.postMessage takes a string");
                }
                return 1;
            };
            let frame = cef::v8_context_get_current_context().and_then(|context| context.frame());
            let message = cef::process_message_create(Some(&IPC_MESSAGE.into()));
            if let (Some(frame), Some(mut message)) = (frame, message) {
                if let Some(arguments) = message.argument_list() {
                    arguments.set_string(0, Some(&text));
                }
                frame.send_process_message(
                    sys::cef_process_id_t::PID_BROWSER.into(),
                    Some(&mut message),
                );
            }
            1
        }
    }
}
