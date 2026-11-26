//! Checks for the build tools that the `cef` feature needs, and writes the stubs that load CEF
//! at run time on Linux.
//!
//! On Windows and macOS, `cef-dll-sys` builds CEF's C++ wrapper with `CMake` and Ninja. Without
//! them it fails with an opaque `CMake` error, so we report the missing tool and how to install
//! it instead. On Linux it builds no wrapper and needs neither.
//!
//! Cargo does not order this script before the one in `cef-dll-sys`, but this one has nothing
//! to compile and nothing to download, so in practice it fails first.

use std::env;
use std::fmt::Write as _;
use std::path::Path;

/// A build tool that the `cef` feature requires.
struct Tool {
    /// The name of the executable.
    name: &'static str,

    /// The environment variable that points at the executable instead, if any.
    env_override: Option<&'static str>,

    /// How to install it with Homebrew.
    brew: &'static str,

    /// How to install it with apt.
    apt: &'static str,

    /// How to install it with winget.
    winget: &'static str,
}

// TODO(emilk): remove `ninja` when `cef-dll-sys` no longer requires it:
// https://github.com/tauri-apps/cef-rs/pull/525
const TOOLS: &[Tool] = &[
    Tool {
        name: "cmake",
        env_override: Some("CMAKE"),
        brew: "brew install cmake",
        apt: "sudo apt install cmake",
        winget: "winget install Kitware.CMake",
    },
    Tool {
        name: "ninja",
        env_override: None,
        brew: "brew install ninja",
        apt: "sudo apt install ninja-build",
        winget: "winget install Ninja-build.Ninja",
    },
];

fn main() {
    cfg_aliases::cfg_aliases! {
        // CEF renders the page off screen, on the desktop platforms that CEF supports.
        cef_backend: { all(feature = "cef", any(target_os = "linux", target_os = "macos", target_os = "windows")) },

        // The page is an embedded view of the platform: an `<iframe>`.
        platform_backend: { target_arch = "wasm32" },
    }

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=DOCS_RS");
    println!("cargo::rerun-if-env-changed=PATH");
    for tool in TOOLS {
        if let Some(env_override) = tool.env_override {
            println!("cargo::rerun-if-env-changed={env_override}");
        }
    }

    let has_cef = env::var_os("CARGO_FEATURE_CEF").is_some();
    let is_wasm = env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|arch| arch == "wasm32");
    // docs.rs builds with `cef/dox`, which builds nothing.
    let is_docs_rs = env::var_os("DOCS_RS").is_some();
    // `cef-dll-sys` builds no C++ wrapper on Linux.
    let is_linux = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "linux");
    if !has_cef || is_wasm {
        return;
    }
    if is_docs_rs {
        // There is no CEF to read the function names from, and nothing is linked.
        println!("cargo::rustc-env=EGUI_WEBVIEW_CEF_ARCHIVE=");
        println!("cargo::rustc-env=EGUI_WEBVIEW_CEF_SHA1=");
        if is_linux {
            write_out_file(
                "cef_stubs.rs",
                "pub const NAMES: [&std::ffi::CStr; 0] = [];\n",
            );
        }
        return;
    }

    if !is_linux {
        for tool in TOOLS {
            if !is_available(tool) {
                let Tool { name, .. } = tool;
                let install = install_command(tool);
                println!(
                    "cargo::error=The `cef` feature of `egui_webview` requires `{name}`, which \
                     was not found on `PATH`. Install it with `{install}`."
                );
            }
        }
    }

    println!("cargo::rerun-if-env-changed=DEP_CEF_DLL_WRAPPER_CEF_DIR");
    // `cef-dll-sys` tells us where it put CEF at build time.
    let Some(cef_dir) = env::var_os("DEP_CEF_DLL_WRAPPER_CEF_DIR") else {
        println!("cargo::error=`cef-dll-sys` didn't report where it put CEF");
        return;
    };
    let cef_dir = Path::new(&cef_dir);
    export_archive(cef_dir);
    if is_linux {
        write_stubs(cef_dir);
    }
}

/// Tells the crate which CEF archive `cef-dll-sys` downloaded, and its SHA-1. The app downloads
/// the same archive when it installs CEF at run time, so it gets the CEF that the bindings were
/// made for.
fn export_archive(cef_dir: &Path) {
    let path = cef_dir.join("archive.json");
    println!("cargo::rerun-if-changed={}", path.display());
    // Without the file, the crate still builds. It just can't install CEF at run time.
    let json = std::fs::read_to_string(&path).unwrap_or_default();
    let name = json_string(&json, "name").unwrap_or_default();
    let sha1 = json_string(&json, "sha1").unwrap_or_default();
    println!("cargo::rustc-env=EGUI_WEBVIEW_CEF_ARCHIVE={name}");
    println!("cargo::rustc-env=EGUI_WEBVIEW_CEF_SHA1={sha1}");
}

/// The string value of `key` in the flat JSON object `json`. It doesn't handle escapes, which
/// file names and hashes don't have.
fn json_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let (_, after_key) = json.split_once(&format!("\"{key}\""))?;
    let (_, after_quote) = after_key.split_once(':')?.1.split_once('"')?;
    Some(after_quote.split_once('"')?.0)
}

/// Writes `contents` to `name` in the build's output directory.
fn write_out_file(name: &str, contents: &str) {
    let Some(out_dir) = env::var_os("OUT_DIR") else {
        return;
    };
    let path = Path::new(&out_dir).join(name);
    if let Err(err) = std::fs::write(&path, contents) {
        println!("cargo::error=failed to write {}: {err}", path.display());
    }
}

/// Writes `cef_stubs.rs`, which `src/chromium/loader.rs` includes. It defines every function
/// that `libcef.so` exports as a stub that jumps to the real function.
///
/// `cef-dll-sys` links `libcef.so`, so a binary would need the library just to start. With the
/// stubs in the binary, nothing refers to the library anymore, and the linker leaves it out.
/// We load it with `dlopen` when the first webview needs it.
fn write_stubs(cef_dir: &Path) {
    // CEF's headers declare the exported functions.
    let include = cef_dir.join("include");
    println!("cargo::rerun-if-changed={}", include.display());
    let mut names = Vec::new();
    if let Err(err) = exported_functions(&include, &mut names) {
        println!(
            "cargo::error=failed to read the CEF headers in {}: {err}",
            include.display()
        );
        return;
    }
    names.sort();
    names.dedup();
    if names.is_empty() {
        println!(
            "cargo::error=found no exported functions in the CEF headers in {}",
            include.display()
        );
        return;
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let mut source = String::new();
    _ = writeln!(
        source,
        "pub const NAMES: [&std::ffi::CStr; {}] = [",
        names.len()
    );
    for name in &names {
        _ = writeln!(source, "    c\"{name}\",");
    }
    _ = writeln!(source, "];\n");
    _ = writeln!(source, "std::arch::global_asm!(");
    _ = writeln!(
        source,
        "    \".pushsection .text.egui_webview_cef_stubs,\\\"ax\\\",@progbits\","
    );
    // `loader.rs` refers to this symbol, so that the linker takes the stubs with the loader.
    for symbol in std::iter::once("egui_webview_cef_stubs").chain(names.iter().map(String::as_str))
    {
        // Hidden, so that the binary doesn't export the stubs. `libcef.so` calls some of its
        // own exports by name, and would otherwise end up in our stubs.
        _ = writeln!(source, "    \".globl {symbol}\",");
        _ = writeln!(source, "    \".hidden {symbol}\",");
        _ = writeln!(source, "    \".type {symbol},@function\",");
        _ = writeln!(source, "    \"{symbol}:\",");
        if symbol == "egui_webview_cef_stubs" {
            continue;
        }
        let offset = 8 * names.iter().position(|name| name == symbol).unwrap_or(0);
        match arch.as_str() {
            "x86_64" => {
                _ = writeln!(
                    source,
                    "    \"jmp qword ptr [rip + {{slots}} + {offset}]\","
                );
            }
            "aarch64" => {
                // x16 is the register that the ABI sets aside for stubs like this one.
                _ = writeln!(source, "    \"adrp x16, {{slots}}\",");
                _ = writeln!(source, "    \"add x16, x16, :lo12:{{slots}}\",");
                _ = writeln!(source, "    \"ldr x16, [x16, #{offset}]\",");
                _ = writeln!(source, "    \"br x16\",");
            }
            _ => {
                println!("cargo::error=the `cef` feature doesn't support `{arch}` on Linux");
                return;
            }
        }
    }
    _ = writeln!(source, "    \".popsection\",");
    _ = writeln!(source, "    slots = sym super::SLOTS,");
    _ = writeln!(source, ");");

    write_out_file("cef_stubs.rs", &source);
}

/// Collects the names of the functions that the headers in `dir` declare with `CEF_EXPORT`.
fn exported_functions(dir: &Path, names: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            exported_functions(&path, names)?;
        } else if path.extension().is_some_and(|extension| extension == "h") {
            let header = String::from_utf8_lossy(&std::fs::read(&path)?).into_owned();
            // A declaration looks like `CEF_EXPORT int cef_initialize(…);`. The return type
            // can be several words, and the name can be on the next line.
            for declaration in header.split("CEF_EXPORT").skip(1) {
                let before_arguments = declaration.split(['(', ';', '{', '#']).next();
                let name = before_arguments.and_then(|text| text.split_whitespace().last());
                if let Some(name) = name.map(|name| name.trim_start_matches('*'))
                    && name.starts_with("cef_")
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    names.push(name.to_owned());
                }
            }
        }
    }
    Ok(())
}

fn is_available(tool: &Tool) -> bool {
    let has_override = tool
        .env_override
        .is_some_and(|env_override| env::var_os(env_override).is_some());

    has_override || is_on_path(tool.name)
}

fn is_on_path(name: &str) -> bool {
    let file_name = format!("{name}{}", env::consts::EXE_SUFFIX);

    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(&file_name).is_file()))
}

/// The install command for the OS that runs the build.
fn install_command(tool: &Tool) -> &'static str {
    let Tool {
        brew, apt, winget, ..
    } = tool;

    match env::consts::OS {
        "macos" => brew,
        "windows" => winget,
        _ => apt,
    }
}
