//! Checks for the build tools that the `cef` feature needs.
//!
//! `cef-dll-sys` builds CEF's C++ wrapper with `CMake` and Ninja. Without them it fails with an
//! opaque `CMake` error, so we report the missing tool and how to install it instead.
//!
//! Cargo does not order this script before the one in `cef-dll-sys`, but this one has nothing
//! to compile and nothing to download, so in practice it fails first.

use std::env;

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
    if !has_cef || is_wasm || is_docs_rs {
        return;
    }

    for tool in TOOLS {
        if !is_available(tool) {
            let Tool { name, .. } = tool;
            let install = install_command(tool);
            println!(
                "cargo::error=The `cef` feature of `egui_webview` requires `{name}`, which was \
                 not found on `PATH`. Install it with `{install}`."
            );
        }
    }
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
