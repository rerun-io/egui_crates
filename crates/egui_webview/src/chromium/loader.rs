//! Loads `libcef.so` at run time on Linux, so that an app starts without it.
//!
//! The `cef` crate calls CEF's C functions (`cef_initialize`, …) by name. `cef-dll-sys` links
//! `libcef.so` for them, and then the app can't start without the library. So we define these
//! functions ourselves, as stubs. Each stub jumps to the address in its slot of [`SLOTS`].
//! [`load`] opens the library and fills the slots with its functions. The linker sees that
//! nothing needs `libcef.so` anymore and leaves it out of the binary.
//!
//! The build script writes the stubs, from the functions that CEF's headers declare.

use std::ffi::c_void;
use std::path::Path;
use std::sync::atomic::{AtomicPtr, Ordering};

use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};

/// `NAMES`, the functions that `libcef.so` exports, and the stub of each one. The stubs are
/// assembly, which counts as unsafe code.
#[expect(unsafe_code)]
mod stubs {
    include!(concat!(env!("OUT_DIR"), "/cef_stubs.rs"));
}
use stubs::NAMES;

/// Where each stub jumps to. The slot of a function has its index in `NAMES`.
///
/// Until [`load`] has run, every stub jumps to [`not_loaded`].
#[expect(clippy::fn_to_numeric_cast_any)] // A slot holds the address of a function.
static SLOTS: [AtomicPtr<c_void>; NAMES.len()] =
    [const { AtomicPtr::new(not_loaded as *mut c_void) }; NAMES.len()];

#[expect(unsafe_code)]
unsafe extern "C" {
    /// The start of the stubs. Using it in [`load`] makes the linker take the stubs whenever
    /// it takes the loader.
    safe static egui_webview_cef_stubs: u8;
}

/// What a stub calls before [`load`] has run, or for a function that the library doesn't have.
extern "C" fn not_loaded() -> ! {
    // Unwinding into CEF's or the `cef` crate's C frames isn't allowed, so don't panic.
    log::error!("egui_webview: a CEF function was called, but CEF doesn't have it or isn't loaded");
    std::process::abort();
}

/// Loads `libcef.so` from `dir` and points the stubs at its functions.
///
/// Call it once, before the first call of a CEF function. The library stays loaded until the
/// process exits.
pub fn load(dir: &Path) -> Result<(), String> {
    std::hint::black_box(&raw const egui_webview_cef_stubs);

    let path = dir.join(super::install::LIBRARY);
    // SAFETY: loading a library runs its initializers. We trust the CEF library that the app
    // points us at as much as one linked at build time.
    #[expect(unsafe_code)]
    let library = unsafe { Library::open(Some(&path), RTLD_NOW | RTLD_LOCAL) }
        .map_err(|err| format!("failed to load {}: {err}", path.display()))?;

    let mut missing = 0;
    for (name, slot) in NAMES.iter().zip(&SLOTS) {
        // SAFETY: we only take the function's address here. The `cef` crate's bindings give it
        // its type when they call the stub.
        #[expect(unsafe_code)]
        let function = unsafe { library.get::<*mut c_void>(*name) };
        match function {
            Ok(function) => slot.store(function.as_raw_ptr(), Ordering::Release),
            Err(_) => missing += 1,
        }
    }
    // CEF never unloads cleanly, so keep the library for the life of the process.
    _ = library.into_raw();

    if 0 < missing {
        // The headers declare functions of every API version, so a few can be missing. A
        // library from another CEF version is rejected later, when the API version is checked.
        log::debug!(
            "egui_webview: {} lacks {missing} CEF functions",
            path.display()
        );
    }
    Ok(())
}
