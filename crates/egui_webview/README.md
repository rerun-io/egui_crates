# egui_webview

Web pages as [egui](https://www.egui.rs/) widgets.

This crate is a work in progress.

The app creates a `WebViewInstance` once, keeps it, and calls `WebViewInstance::ui` in every pass in which the page should be shown.
After the pass, it places the page from eframe's output hook:

```rust,ignore
impl eframe::App for MyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.webview.ui(ui, ui.available_size());
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webview.place(ctx, &output.platform_output.embedded_views);
    }
}
```

Tab and Shift-Tab move the keyboard focus between egui widgets and the page's elements.
`examples/minimal_instance.rs` is a complete app of this shape.

## Initialization

Create an `egui_webview::Runtime` at the start of `main`, right after initializing logging, and keep it until `eframe::run_native` has returned:

```rust,ignore
fn main() -> eframe::Result {
    env_logger::init();
    let _runtime = egui_webview::Runtime::new();
    eframe::run_native(/* … */)
}
```

Natively, creating a webview without a running `Runtime` fails with `Error::NoRuntime`, which `WebView` shows in the page's rect. On the web, `Runtime` does nothing. Natively, only one `Runtime` may be alive at a time, and creating a second one panics. Live webviews keep CEF running. After the `Runtime` and the last webview are dropped, CEF shuts down once all browsers have closed and deletes its temporary files.

## Showing a page without keeping it

A `WebViewInstance` isn't `Send`, so not every piece of code can keep one.
`WebView` shows a page with one call and keeps nothing:

```rust,ignore
let url = egui_webview::Url::parse("https://docs.rs/egui")?;
let response = egui_webview::WebView::new(egui::Id::unique("docs"))
    .url(&url)
    .show(ui);
```

The pages then live in a `WebViews` that the app owns and updates from eframe's output hook:

```rust,ignore
impl eframe::App for MyApp {
    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webviews.update(ctx, output);
    }
}
```

`examples/minimal.rs` is a complete app of this shape.

* The id is global. The same id shows the same page from anywhere in the app.
* A page lives only while it is shown. Call `WebView::keep_alive` in every frame to keep a hidden page.
* `WebView::reload`, `WebView::load` and the other commands take the id, so they work from anywhere too.

## Running the examples

Natively:

```sh
cargo run -p egui_webview --example minimal
cargo run -p egui_webview --example minimal_instance
```

The examples run on the web with [Trunk](https://github.com/trunk-rs/trunk):

```sh
cd crates/egui_webview
trunk serve --example minimal
trunk serve --example minimal_instance
```

`crates/egui_webview_demo` is a bigger demo, with a browser bar, pages in egui windows and a log of the pages' events.

## Backends

* Web: an `<iframe>` below egui's canvas.
    * egui clears its rect to transparent so the page shows through.
    * egui windows and popups above a page paint over it and receive the pointer input there.
* Any native platform, with the `cef` feature: Chromium, through [CEF](https://github.com/chromiumembedded/cef), renders the page off screen into an egui texture, so it works with every egui renderer.
  `WebViewInstance::new` and `WebViewInstance::new_cef` use it on every native platform.
  `WebViewInstance::new_cef_with_options` also takes `CefOptions`: the page's frame rate, and the background color of pages that set none, which is white by default.


When a page asks for a new window (`window.open`, or a link with `target="_blank"`), the CEF backend opens nothing and sends `WebViewEvent::NewWindowRequested` instead. The app decides what to do with the URL, for example load it with `load_url` or open it in the system browser. On the web, the browser handles new windows itself.

Every backend gives the page egui's theme as its `prefers-color-scheme`, and updates it when the theme changes.
So pages that follow the OS theme, such as ones built with Pico CSS or Tailwind's `dark:`, follow the app's theme instead.

## CEF

The [`cef`](https://crates.io/crates/cef) crate downloads CEF (about 1 GB) the first time it is built.
On Windows and macOS it also builds a small C++ wrapper library, which requires CMake and Ninja; on Linux it needs neither, and loads `libcef.so` at run time.
Set `CEF_PATH` to share one download between projects.

CEF launches its helper processes (renderer, GPU, …) from the app's own executable, so `main` also runs in each of them. That is why `egui_webview::Runtime::new` must run at the start of `main`, before anything but logging: in a helper process, it hands the process over to CEF and exits when the helper is done. In the app's own process, it returns the `Runtime`.

Creating the `Runtime` is cheap. It starts no engine, process, or thread. CEF itself starts when the first CEF webview is created, so there is no reason to create the `Runtime` later than at the start of `main`.

`egui_webview::Runtime::with_cef_options` takes `CefInitOptions`, which apply to the whole process. By default, nothing persists: the browser profile (cookies, `localStorage`, the HTTP cache) lives in memory. Set `CefInitOptions::cache_dir` to keep the profile in a directory across restarts. Only one running process can use a directory at a time.

`egui_webview::run_cef_subprocess` does the same as `Runtime::with_cef_options` without exiting. It returns `CefProcess::Helper` with the helper's exit code, or `CefProcess::App` with the `Runtime`.

### Shipping an app without CEF

CEF is over 1 GB unpacked, so you may not want it in your installer.
On Linux and macOS, the app starts without CEF's files, and CEF is loaded when the first webview needs it.
On Windows, the CEF library is still linked at build time.

* `CefInitOptions::cef_dir` is a folder of your choice for CEF, such as your app's cache folder. CEF for this build goes in `<cef_dir>/<CEF version>/cef_<os>_<arch>`. CEF is also found next to the executable, and where the `cef` crate put it at build time.
* `egui_webview::cef_status` says whether CEF is there.
* With the `cef-download` feature, `egui_webview::cef_install` downloads the CEF build that the crate was built with into `cef_dir`, on its own thread. For Linux on x86-64, that is about 330 MB, and 1.5 GB unpacked. Nothing is downloaded unless you call it, so ask the user first.
* `egui_webview::uses_cef` says whether `WebView` and `WebViewInstance::new` use CEF at all. On a platform with a system webview they don't, and `cef_status` alone would report a missing CEF that nothing needs.
* A `WebView` whose page can't be created because CEF is missing says so in the page's place, offers the download, shows its progress, and shows the page once CEF is there. To show your own UI in its place, turn that off with `WebView::cef_install_ui(false)` and look at `WebViewResponse::error`.
* `WebViewInstance::new` returns `Error::CefNotInstalled`. Show `egui_webview::cef_install_ui`, which is the UI that `WebView` shows, and create the page once `cef_status` is `CefStatus::Installed`. The `cef_install` example does this.

The helper processes look for CEF in the same places, so pass the same `cef_dir` in every run of `main`.

#### Where to put CEF

Use a subfolder of your app in the platform's cache folder. CEF can be downloaded again, and a folder of your own makes it clear who owns the files and what to delete on uninstall. The library has no default folder. The examples do this with the [`dirs`](https://crates.io/crates/dirs) crate:

```rust
let cef_dir = dirs::cache_dir().map(|dir| dir.join("my_app").join("cef"));
let _runtime = egui_webview::Runtime::with_cef_options(egui_webview::CefInitOptions {
    cef_dir,
    ..Default::default()
});
```

That is `$XDG_CACHE_HOME` or `~/.cache` on Linux, `~/Library/Caches` on macOS, and `%LOCALAPPDATA%` on Windows.

#### `cef_dir` and `CEF_PATH`

They are two different things:

* `CefInitOptions::cef_dir` is your app's folder, set in code. It is searched first, and it is the only place `cef_install` writes to.
* `CEF_PATH` is an environment variable of the `cef` crate. At build time it says where the build keeps its CEF, instead of in `target`. At run time it is searched last, as a help for development. Without it, the last place searched is the build's own copy in `target`.

When `CEF_PATH` is set, the build's copy in `target` isn't searched. That is how `CEF_PATH=/nonexistent` hides it below.

Both use the same layout: CEF is in `<folder>/<CEF version>/cef_<os>_<arch>`, or directly in the folder. So one folder can be the build's `CEF_PATH` and an app's `cef_dir`.

A build on your machine finds CEF by itself, and nothing is downloaded during normal development. If you built with `CEF_PATH`, set it when you run, too. An app on a user's machine has neither, and finds CEF in `cef_dir` or next to the executable.

#### Trying the download

To see what a user without CEF sees, hide the build's copy. From the workspace root:

```sh
cargo build -p egui_webview --features cef-download --example minimal
CEF_PATH=/nonexistent target/debug/examples/minimal
```

Run the binary itself. With `cargo run`, the `CEF_PATH` would also reach the build of the `cef` crate, which would then try to download CEF to `/nonexistent`.

* Click "Download Chromium", or set `EGUI_WEBVIEW_CEF_INSTALL=1` to start the download without a click. The second is a switch of the examples for tests, not of the library.
* The files land in the examples' cache folder, such as `~/.cache/egui_webview_examples/cef/156.0.26/cef_linux_x86_64` on Linux. Delete `~/.cache/egui_webview_examples` to try again.
* On Linux, `XDG_CACHE_HOME=/tmp/some_folder` moves the cache folder for one run. It moves every cache of the process, so fonts and shaders are cached there again, and the first run starts a bit slower.
* `CEF_DOWNLOAD_URL=http://127.0.0.1:9` makes the download fail, to see the error and "Try again". The variable replaces CEF's build server with a mirror of your own. Only set it when you run the binary. The `cef` crate reads it at build time too.
* `cargo test -p egui_webview --features cef-download --lib -- --ignored install` runs the download and install into a temporary folder, without a window.

### Sandbox

By default, CEF's helper processes run without Chromium's sandbox. A page that exploits a bug in Chromium's renderer then runs with the rights of the app's user, so only show pages you trust, or turn the sandbox on: enable the `cef-sandbox` feature and set `CefInitOptions::sandbox` to `true`. The feature alone changes nothing. Where the sandbox can't work, CEF fails to start:

* Linux: needs unprivileged user namespaces, or CEF's SUID `chrome-sandbox` helper.
* macOS: needs an app bundle with CEF's helper apps.
* Windows: not supported.

### Known limitations

* Pages are rendered on the CPU and uploaded as textures.
* JavaScript dialogs, file pickers and drag and drop aren't supported yet.
* While a CEF webview exists, egui repaints at least every 100 ms, because CEF's message loop runs in egui's passes.
