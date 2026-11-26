//! CEF's process-wide initialization, the entry point of its helper processes, and the external
//! message pump that egui's passes drive.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    App, Browser, BrowserProcessHandler, CefString, ImplApp, ImplBrowserProcessHandler,
    ImplCommandLine as _, ImplFrame as _, ImplRenderProcessHandler, RenderProcessHandler, WrapApp,
    WrapBrowserProcessHandler, WrapRenderProcessHandler, sys, wrap_app,
    wrap_browser_process_handler, wrap_render_process_handler,
};
use cef::{
    ImplDomnode as _, ImplListValue as _, ImplProcessMessage as _, ImplV8Context as _,
    ImplV8Handler, ImplV8Value as _, V8Handler, V8Value, WrapV8Handler, wrap_v8_handler,
};
use egui::Ui;
use tempfile::TempDir;

use super::context::Context;
use crate::{Error, Runtime, runtime::Engine};

/// The name of the process message that brings the text of a page's
/// `window.ipc.postMessage(text)` from the render process to the browser process.
pub const IPC_MESSAGE: &str = "egui_webview_ipc";

/// The name of the process message that tells the browser process whether the element that
/// gained focus in a frame is editable. Its one argument is that bool.
pub const FOCUSED_NODE_MESSAGE: &str = "egui_webview_focused_node";

/// The longest time between two calls to `cef::do_message_loop_work` while a webview exists.
///
/// With an external message pump, CEF schedules its work through
/// `on_schedule_message_pump_work`, but it can also have work pending that it didn't schedule.
pub const MAX_PUMP_DELAY: Duration = Duration::from_millis(100);

/// How long [`shutdown`] waits for the browsers to close.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Where CEF is in its life, which happens once per process.
#[expect(clippy::disallowed_types)] // egui's `Mutex` has no `const` constructor for a `static`.
static STATE: std::sync::Mutex<State> = std::sync::Mutex::new(State::Unchecked);

/// Whether the last owner timed out waiting for an asynchronous browser close.
static SHUTDOWN_PENDING: AtomicBool = AtomicBool::new(false);

/// Whether CEF is initialized and not shut down, so that egui's passes may pump its message loop.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The number of browsers that CEF created and hasn't closed yet.
static OPEN_BROWSERS: AtomicUsize = AtomicUsize::new(0);

/// The live pages, whose egui contexts [`wake`] repaints when CEF schedules work.
///
/// Weak, so that an egui context isn't kept alive after its last page is gone.
#[expect(clippy::disallowed_types)] // egui's `Mutex` has no `const` constructor for a `static`.
static PAGES: std::sync::Mutex<Vec<Weak<Context>>> = std::sync::Mutex::new(Vec::new());

/// Options for CEF that apply to the whole process. Pass them to [`Runtime::with_cef_options`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CefInitOptions {
    /// The directory of the browser profile: cookies, `localStorage`, the HTTP cache, and so on.
    ///
    /// With `Some`, the profile persists in this directory across restarts of the app. Only one
    /// running process can use a directory at a time.
    ///
    /// With `None`, the default, the profile lives in memory and nothing persists. CEF still
    /// needs a directory for some files. It gets a new one in the system's temporary directory,
    /// which is deleted when CEF shuts down.
    pub cache_dir: Option<PathBuf>,

    /// Whether CEF's helper processes run in Chromium's sandbox. The default is `false`.
    ///
    /// Without the sandbox, a page that exploits a bug in Chromium runs with the rights of the
    /// app's user. `true` needs the `cef-sandbox` feature. On Linux, it also needs unprivileged
    /// user namespaces or CEF's SUID `chrome-sandbox`. On macOS, the app must run from an app
    /// bundle with CEF's helper apps. Windows isn't supported. Where the sandbox can't work, CEF
    /// fails to start.
    pub sandbox: bool,

    /// The folder with CEF's files, such as `libcef.so` and `icudtl.dat` on Linux, or the
    /// `Chromium Embedded Framework.framework` on macOS.
    ///
    /// CEF is loaded from here when the first webview needs it, so the app starts without
    /// it. If the folder doesn't have CEF, or with `None`, the default, we look next to the
    /// executable, and then where the `cef` crate put CEF at build time.
    ///
    /// Windows ignores this. There, the CEF library is linked at build time.
    pub cef_dir: Option<PathBuf>,
}

/// What [`crate::run_cef_subprocess`] found this process to be.
#[must_use]
#[derive(Debug)]
pub enum CefProcess {
    /// Chromium started this process as a CEF helper process, which has finished. Exit with
    /// `exit_code`.
    Helper {
        /// The exit code of the helper process.
        exit_code: i32,
    },

    /// This is the app's own process. Start the app, and keep the runtime until
    /// `eframe::run_native` returns.
    App(Runtime),
}

/// Where CEF is in its life.
enum State {
    /// [`run_subprocess`] hasn't run yet.
    Unchecked,

    /// CEF starts with these options when the first page needs it.
    Ready {
        options: CefInitOptions,

        /// CEF starts on the thread that created the [`Runtime`].
        thread: ThreadId,
    },

    Running {
        /// CEF shuts down on the thread that initialized it.
        thread: ThreadId,

        /// The cache directory that [`shutdown`] deletes, if it was made for this process.
        temp_dir: Option<TempDir>,
    },

    /// CEF failed to start, and can't be started again.
    Failed(String),

    /// CEF has shut down, and can't be started again.
    ShutDown,
}

/// Runs this process as a CEF helper process and exits, if Chromium started it as one. Otherwise
/// it returns the [`Runtime`]. See [`Runtime::with_cef_options`].
pub fn init(options: CefInitOptions) -> Runtime {
    match run_subprocess(options) {
        CefProcess::Helper { exit_code } => {
            #[expect(clippy::exit)] // A helper process has nothing of the app to unwind.
            std::process::exit(exit_code);
        }
        CefProcess::App(runtime) => runtime,
    }
}

/// Runs this process as a CEF helper process, if Chromium started it as one, and returns its exit
/// code. Otherwise it keeps `options` for when the first webview starts CEF.
///
/// This is [`Runtime::with_cef_options`] without the exit. Call it at the start of `main`:
///
/// ```no_run
/// let _runtime = match egui_webview::run_cef_subprocess(Default::default()) {
///     egui_webview::CefProcess::Helper { exit_code } => std::process::exit(exit_code),
///     egui_webview::CefProcess::App(runtime) => runtime,
/// };
/// // … run the app
/// ```
pub fn run_subprocess(options: CefInitOptions) -> CefProcess {
    let mut state = lock_state();
    if !matches!(*state, State::Unchecked) {
        drop(state); // `Runtime::start` may panic.
        return CefProcess::App(Runtime::start());
    }
    // The app itself doesn't load CEF here. That waits until the first webview needs it.
    if is_helper() {
        let exit_code = run_helper(&options).unwrap_or_else(|err| {
            log::error!("egui_webview: {err}");
            1
        });
        return CefProcess::Helper { exit_code };
    }

    *state = State::Ready {
        options,
        thread: std::thread::current().id(),
    };
    drop(state); // `Runtime::start` may panic.
    CefProcess::App(Runtime::start())
}

/// Runs this process as a CEF helper process, and returns its exit code.
fn run_helper(options: &CefInitOptions) -> Result<i32, String> {
    let args = cef::args::Args::new();
    // macOS helpers must enter the sandbox before loading the CEF framework.
    #[cfg(all(target_os = "macos", feature = "cef-sandbox"))]
    let _sandbox = helper_sandbox(options, &args)?;

    // The app loaded CEF before it started this helper, so the helper finds it in the same
    // place.
    load_library(options).map_err(StartError::into_message)?;
    let mut app = EguiApp::new(options.cache_dir.is_none());
    let exit_code = cef::execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    // CEF returns -1 for the browser process, which a process with `--type` never is.
    Ok(exit_code.max(0))
}

/// Whether Chromium launched this executable as a helper.
///
/// This is the test that `cef::execute_process` does itself: a process with a `--type` switch
/// is a helper, and one without is the browser process. See `CefMainRunner::RunAsHelperProcess`:
/// <https://github.com/chromiumembedded/cef/blob/8078/libcef/browser/main_runner.cc#L198-L202>
///
/// So an app can't take a `--type=…` argument of its own, with or without this function.
fn is_helper() -> bool {
    std::env::args_os().any(|arg| arg.as_encoded_bytes().starts_with(b"--type="))
}

/// Starts the sandbox in a helper process on macOS, if `options` ask for it. The sandbox lasts
/// until it is dropped.
#[cfg(all(target_os = "macos", feature = "cef-sandbox"))]
fn helper_sandbox(
    options: &CefInitOptions,
    args: &cef::args::Args,
) -> Result<Option<cef::sandbox::Sandbox>, String> {
    if !options.sandbox {
        return Ok(None);
    }
    // The path that `cef::sandbox::Sandbox::new` loads, which panics if it is missing.
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let library = exe.parent().map(|dir| {
        dir.join("../../../Chromium Embedded Framework.framework/Libraries/libcef_sandbox.dylib")
    });
    if !library.is_some_and(|library| library.exists()) {
        return Err(
            "the CEF sandbox needs the helper to run from a helper app in an app bundle".to_owned(),
        );
    }
    let mut sandbox = cef::sandbox::Sandbox::new();
    sandbox.initialize(args.as_main_args());
    Ok(Some(sandbox))
}

fn lock_state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The CEF framework on macOS, in the folder with CEF's files.
#[cfg(target_os = "macos")]
const FRAMEWORK: &str = "Chromium Embedded Framework.framework/Chromium Embedded Framework";

/// Why CEF didn't start.
enum StartError {
    /// CEF's files weren't found. The app can install them and try again.
    NotInstalled(String),

    /// CEF can't start in this process anymore.
    Failed(String),
}

impl StartError {
    fn into_message(self) -> String {
        match self {
            Self::NotInstalled(err) | Self::Failed(err) => err,
        }
    }
}

impl From<String> for StartError {
    fn from(err: String) -> Self {
        Self::Failed(err)
    }
}

/// Loads the CEF library, and selects the CEF API version that these bindings were generated
/// for. Both come before every other CEF call, and happen once per process.
///
/// Returns the folder with CEF's files.
fn load_library(options: &CefInitOptions) -> Result<&'static Path, StartError> {
    static CEF_DIR: OnceLock<PathBuf> = OnceLock::new();
    if let Some(dir) = CEF_DIR.get() {
        return Ok(dir);
    }
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let dir = find_cef(options, &exe).map_err(StartError::NotInstalled)?;
    cfg_select! {
        target_os = "linux" => { super::loader::load(&dir)?; }
        target_os = "macos" => { load_framework(&dir)?; }
        // On Windows, the CEF library is linked at build time.
        _ => {}
    }
    _ = cef::api_hash(sys::CEF_API_VERSION_LAST, 0);
    Ok(CEF_DIR.get_or_init(|| dir))
}

/// Finds the folder with CEF's files. See [`CefInitOptions::cef_dir`] for where we look.
fn find_cef(options: &CefInitOptions, exe: &Path) -> Result<PathBuf, String> {
    let exe_dir = exe.parent().ok_or("no executable directory")?;
    // In an app bundle on macOS, the framework is in `Contents/Frameworks`. That is
    // `../Frameworks` from the main executable in `Contents/MacOS`, and `../../..` from a
    // helper's executable in `Contents/Frameworks/<App> Helper.app/Contents/MacOS`.
    #[cfg(target_os = "macos")]
    let (library, candidates) = (
        FRAMEWORK,
        vec![
            Some(exe_dir.join("../Frameworks")),
            Some(exe_dir.join("../../..")),
            options.cef_dir.clone(),
            sys::get_cef_dir(),
        ],
    );
    #[cfg(target_os = "linux")]
    let (library, candidates) = (
        super::loader::LIBRARY,
        vec![
            options.cef_dir.clone(),
            Some(exe_dir.to_owned()),
            sys::get_cef_dir(),
        ],
    );
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let (library, candidates) = {
        _ = options;
        (
            "libcef.dll",
            vec![Some(exe_dir.to_owned()), sys::get_cef_dir()],
        )
    };
    // CEF needs absolute paths.
    let candidates: Vec<PathBuf> = candidates
        .into_iter()
        .flatten()
        .filter_map(|dir| std::path::absolute(dir).ok())
        .collect();
    let found = candidates.iter().find(|dir| dir.join(library).exists());
    if let Some(dir) = found {
        return Ok(dir.clone());
    }
    if cfg!(windows) {
        // The process wouldn't run without the library, so Windows found it some other way.
        return Ok(exe_dir.to_owned());
    }
    let searched: Vec<String> = candidates
        .iter()
        .map(|dir| dir.display().to_string())
        .collect();
    Err(format!(
        "CEF isn't installed. Looked for `{library}` in: {}",
        searched.join(", ")
    ))
}

/// Loads the CEF framework on macOS from `dir`.
#[cfg(target_os = "macos")]
fn load_framework(dir: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt as _;

    let path = dir.join(FRAMEWORK);
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
/// [`Error::NoRuntime`] if no [`Runtime`] runs on this thread. [`Error::Backend`] if CEF runs, or
/// is to start, on another thread, if the CEF library can't be loaded, if CEF fails to
/// initialize, or if CEF has shut down. The first failure is kept, so later calls fail in the
/// same way.
pub fn initialize(ctx: &egui::Context) -> Result<Engine, Error> {
    let engine = Runtime::engine()?;
    let mut state = lock_state();
    if let State::Ready { thread, .. } | State::Running { thread, .. } = &*state
        && *thread != std::thread::current().id()
    {
        return Err(Error::backend(
            "CEF: create webviews on the thread that created the `egui_webview::Runtime`",
        ));
    }
    if let State::Ready { options, .. } = &*state {
        let options = options.clone();
        *state = match start(&options) {
            Ok(temp_dir) => {
                RUNNING.store(true, Ordering::Relaxed);
                State::Running {
                    thread: std::thread::current().id(),
                    temp_dir,
                }
            }
            // Stay ready, so that the next webview tries again after the app installed CEF.
            Err(StartError::NotInstalled(err)) => return Err(Error::backend(err)),
            Err(StartError::Failed(err)) => State::Failed(err),
        };
    }
    let err = match &*state {
        State::Running { .. } => {
            // egui ignores a second plugin of the same type.
            ctx.add_plugin(CefPlugin);
            return Ok(engine);
        }
        // CEF starts its helper processes from this executable. Refuse to start them before the
        // app has shown that its `main` handles being one.
        State::Unchecked | State::Ready { .. } => {
            "create the `egui_webview::Runtime` with `Runtime::new` first in `main`"
        }
        State::Failed(err) => err.as_str(),
        State::ShutDown => {
            "CEF has shut down, because the `egui_webview::Runtime` was dropped. \
             Keep it until `eframe::run_native` returns"
        }
    };
    Err(Error::backend(format!("CEF: {err}")))
}

/// Initializes CEF with `options`. Returns the cache directory to delete at shutdown, if any.
fn start(options: &CefInitOptions) -> Result<Option<TempDir>, StartError> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    check_sandbox(options.sandbox, &exe)?;
    let cef_dir = load_library(options)?;

    let mut settings = cef::Settings {
        windowless_rendering_enabled: 1,
        external_message_pump: 1,
        // CEF's SIGINT and SIGTERM handlers terminate `NSApp` from inside
        // `cef::do_message_loop_work`, which makes winit panic on its re-entered event handler.
        // Signals belong to the app.
        disable_signal_handlers: 1,
        no_sandbox: i32::from(!options.sandbox),
        log_severity: sys::cef_log_severity_t::LOGSEVERITY_WARNING.into(),
        ..Default::default()
    };
    // In a sandboxed app bundle on macOS, CEF finds the helper apps by itself.
    if !options.sandbox || !cfg!(target_os = "macos") {
        settings.browser_subprocess_path = exe.to_string_lossy().as_ref().into();
    }
    let temp_dir = if let Some(dir) = &options.cache_dir {
        // CEF needs an absolute path, and the profile in `cache_path` to persist it.
        let dir = std::path::absolute(dir).map_err(|err| err.to_string())?;
        settings.root_cache_path = dir.to_string_lossy().as_ref().into();
        settings.cache_path = dir.to_string_lossy().as_ref().into();
        None
    } else {
        // Without a `cache_path`, the profile lives in memory.
        let dir = temp_cache_dir().map_err(|err| format!("failed to create CEF cache: {err}"))?;
        settings.root_cache_path = dir.path().to_string_lossy().as_ref().into();
        Some(dir)
    };
    resource_paths(cef_dir, &exe, &mut settings);

    // CEF keeps pointers into the arguments for the life of the process.
    let args = Box::leak(Box::new(cef::args::Args::new()));
    let mut app = EguiApp::new(options.cache_dir.is_none());
    if cef::initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) == 1
    {
        Ok(temp_dir)
    } else {
        Err("CEF failed to initialize".to_owned().into())
    }
}

/// Fails if `sandbox` asks for the sandbox, and it can't work here.
fn check_sandbox(sandbox: bool, exe: &Path) -> Result<(), String> {
    if !sandbox {
        return Ok(());
    }
    if !cfg!(feature = "cef-sandbox") {
        return Err("`CefInitOptions::sandbox` needs the `cef-sandbox` feature".to_owned());
    }
    cfg_select! {
        windows => {
            _ = exe;
            Err("the CEF sandbox isn't supported on Windows".to_owned())
        }
        target_os = "macos" => {
            if in_app_bundle(exe) {
                Ok(())
            } else {
                Err("the CEF sandbox needs an app bundle with CEF's helper apps on macOS"
                    .to_owned())
            }
        }
        _ => {
            _ = exe;
            Ok(())
        }
    }
}

/// Whether `exe` is the main executable of an app bundle, with CEF in its `Frameworks`.
#[cfg(target_os = "macos")]
fn in_app_bundle(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|dir| dir.join("../Frameworks").exists())
}

/// Shuts CEF down, if it runs. Called when the [`Runtime`] and the last webview are dropped.
///
/// If closing takes longer than [`SHUTDOWN_TIMEOUT`], a later message pump finishes shutdown.
/// The temporary cache directory is deleted after shutdown. CEF can't start again in this process.
pub fn shutdown() {
    let mut state = lock_state();
    let this_thread = std::thread::current().id();
    // CEF belongs to the thread that created the first `Runtime`. Dropping the runtime of
    // another thread leaves CEF alone.
    if let State::Ready { thread, .. } | State::Running { thread, .. } = &*state
        && *thread != this_thread
    {
        return;
    }
    let State::Running { temp_dir, .. } = &mut *state else {
        // CEF never started, so there is nothing to shut down. It can't start later either.
        *state = State::ShutDown;
        return;
    };

    // Closing a browser takes a few turns of the message loop.
    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    while 0 < OPEN_BROWSERS.load(Ordering::Relaxed) {
        if deadline < Instant::now() {
            log::warn!(
                "egui_webview: deferring CEF shutdown until {} browsers finish closing",
                OPEN_BROWSERS.load(Ordering::Relaxed)
            );
            SHUTDOWN_PENDING.store(true, Ordering::Relaxed);
            return;
        }
        cef::do_message_loop_work();
        std::thread::sleep(Duration::from_millis(5));
    }
    RUNNING.store(false, Ordering::Relaxed);
    cef::shutdown();
    let temp_dir = temp_dir.take();
    *state = State::ShutDown;

    if let Some(dir) = temp_dir
        && let Err(err) = dir.close()
    {
        log::warn!("egui_webview: failed to delete temporary CEF cache: {err}");
    }
}

/// Counts a browser that CEF created. Call from `on_after_created`.
pub fn browser_opened() {
    OPEN_BROWSERS.fetch_add(1, Ordering::Relaxed);
}

/// Counts a browser that CEF closed. Call from `on_before_close`.
pub fn browser_closed() {
    OPEN_BROWSERS.fetch_sub(1, Ordering::Relaxed);
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

/// A new cache directory for this process, so that two running instances of an app don't share
/// one. [`shutdown`] deletes it.
fn temp_cache_dir() -> std::io::Result<TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("egui_webview_");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder.tempdir()
}

/// Points CEF at its resources in `cef_dir`, unless they are where CEF looks by itself, as in a
/// packaged app.
fn resource_paths(cef_dir: &Path, exe: &Path, settings: &mut cef::Settings) {
    cfg_select! {
        target_os = "macos" => {
            if !in_app_bundle(exe) {
                settings.framework_dir_path = cef_dir
                    .join("Chromium Embedded Framework.framework")
                    .to_string_lossy()
                    .as_ref()
                    .into();
            }
        }
        _ => {
            if exe.parent() != Some(cef_dir) {
                settings.resources_dir_path = cef_dir.to_string_lossy().as_ref().into();
                settings.locales_dir_path =
                    cef_dir.join("locales").to_string_lossy().as_ref().into();
            }
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
        if RUNNING.load(Ordering::Relaxed) {
            cef::do_message_loop_work();
            if SHUTDOWN_PENDING.load(Ordering::Relaxed)
                && OPEN_BROWSERS.load(Ordering::Relaxed) == 0
            {
                SHUTDOWN_PENDING.store(false, Ordering::Relaxed);
                shutdown();
            }
        }
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
    struct EguiApp {
        ephemeral_profile: bool,
    }

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
            // An in-memory profile has no cookies to encrypt on disk.
            if cfg!(target_os = "macos") && self.ephemeral_profile {
                command_line.append_switch(Some(&"use-mock-keychain".into()));
            }
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

        /// Tells the browser process in a [`FOCUSED_NODE_MESSAGE`] whether the element that
        /// gained focus in `frame` is editable. `node` is `None` when no element has focus.
        fn on_focused_node_changed(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut cef::Frame>,
            node: Option<&mut cef::Domnode>,
        ) {
            let editable = node.is_some_and(|node| node.is_editable() != 0);
            let message = cef::process_message_create(Some(&FOCUSED_NODE_MESSAGE.into()));
            if let (Some(frame), Some(mut message)) = (frame, message) {
                if let Some(arguments) = message.argument_list() {
                    arguments.set_bool(0, editable.into());
                }
                frame.send_process_message(
                    sys::cef_process_id_t::PID_BROWSER.into(),
                    Some(&mut message),
                );
            }
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
