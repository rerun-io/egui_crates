//! Where CEF's files are on this machine, and the download that puts them there.
//!
//! The app starts without CEF. It gives us a folder in [`CefInitOptions::cef_dir`], and CEF for
//! this build lives in `<cef_dir>/<CEF version>/cef_<os>_<arch>`. That is the layout that the
//! `cef` crate uses for `CEF_PATH` at build time, so the two can share a folder.
//!
//! Nothing is downloaded unless the app calls [`install`].

use std::path::{Path, PathBuf};

use super::runtime::CefInitOptions;

/// The archive with the CEF build that the `cef` crate's bindings were made for, such as
/// `cef_binary_156.0.26+g8ea0c73+chromium-156.0.8078.88_linux64_minimal.tar.bz2`.
///
/// Empty if the build didn't know the archive.
const ARCHIVE: &str = env!("EGUI_WEBVIEW_CEF_ARCHIVE");

/// The SHA-1 of [`ARCHIVE`], as CEF's build server published it when this crate was built.
#[cfg(feature = "cef-download")]
const ARCHIVE_SHA1: &str = env!("EGUI_WEBVIEW_CEF_SHA1");

/// The file in the folder with CEF's files that tells us CEF is there.
#[cfg(target_os = "macos")]
pub const LIBRARY: &str = "Chromium Embedded Framework.framework/Chromium Embedded Framework";

/// The file in the folder with CEF's files that tells us CEF is there.
#[cfg(target_os = "linux")]
pub const LIBRARY: &str = "libcef.so";

/// The file in the folder with CEF's files that tells us CEF is there.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub const LIBRARY: &str = "libcef.dll";

/// Whether CEF is on this machine. See [`crate::cef_status`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CefStatus {
    /// CEF's files weren't found. A webview can't show a page until they are there.
    ///
    /// With the `cef-download` feature, `cef_install` downloads them.
    NotInstalled,

    /// `cef_install` is downloading and unpacking CEF.
    Installing {
        /// How many bytes of the download have arrived.
        downloaded: u64,

        /// The size of the download in bytes, once the server has told us.
        total: Option<u64>,
    },

    /// CEF's files are there.
    Installed,

    /// `cef_install` failed, with this message. Calling it again starts over.
    InstallFailed(String),
}

/// The CEF version in [`ARCHIVE`], such as `156.0.26`.
fn version() -> Option<&'static str> {
    let version = ARCHIVE.strip_prefix("cef_binary_")?.split('+').next()?;
    (!version.is_empty()).then_some(version)
}

/// Where CEF for this build is, or gets installed, in the app's folder `cef_dir`.
fn versioned_dir(cef_dir: &Path) -> Option<PathBuf> {
    use std::env::consts::{ARCH, OS};
    Some(cef_dir.join(version()?).join(format!("cef_{OS}_{ARCH}")))
}

/// Finds the folder with CEF's files. See [`CefInitOptions::cef_dir`] for where we look.
pub fn find(options: &CefInitOptions, exe: &Path) -> Result<PathBuf, String> {
    let exe_dir = exe.parent().ok_or("no executable directory")?;
    let from_options = options
        .cef_dir
        .iter()
        .flat_map(|dir| [versioned_dir(dir), Some(dir.clone())]);
    // In an app bundle on macOS, the framework is in `Contents/Frameworks`. That is
    // `../Frameworks` from the main executable in `Contents/MacOS`, and `../../..` from a
    // helper's executable in `Contents/Frameworks/<App> Helper.app/Contents/MacOS`.
    let next_to_exe = if cfg!(target_os = "macos") {
        [
            Some(exe_dir.join("../Frameworks")),
            Some(exe_dir.join("../../..")),
        ]
    } else {
        [Some(exe_dir.to_owned()), None]
    };
    // CEF needs absolute paths.
    let candidates: Vec<PathBuf> = from_options
        .chain(next_to_exe)
        .chain([cef::sys::get_cef_dir()])
        .flatten()
        .filter_map(|dir| std::path::absolute(dir).ok())
        .collect();
    if let Some(dir) = candidates.iter().find(|dir| dir.join(LIBRARY).exists()) {
        return Ok(dir.clone());
    }
    if cfg!(windows) {
        // On Windows, the CEF library is linked at build time. The process wouldn't run
        // without it, so Windows found it some other way.
        return Ok(exe_dir.to_owned());
    }
    let searched: Vec<String> = candidates
        .iter()
        .map(|dir| dir.display().to_string())
        .collect();
    Err(format!(
        "CEF isn't installed. Looked for `{LIBRARY}` in: {}",
        searched.join(", ")
    ))
}

#[cfg(feature = "cef-download")]
pub use download::{install, status};

/// Without the `cef-download` feature, nothing installs CEF, so it is there or it isn't.
#[cfg(not(feature = "cef-download"))]
pub fn status() -> Option<CefStatus> {
    None
}

#[cfg(feature = "cef-download")]
mod download {
    use std::fs::File;
    use std::io::{BufReader, Read};
    use std::path::{Component, Path, PathBuf};

    use super::{ARCHIVE, ARCHIVE_SHA1, CefStatus, LIBRARY, versioned_dir};

    /// CEF's build server, which the `cef` crate also downloads from at build time.
    const DOWNLOAD_URL: &str = "https://cef-builds.spotifycdn.com";

    /// What [`install`] is doing. `None` until it is called.
    #[expect(clippy::disallowed_types)] // egui's `Mutex` has no `const` constructor.
    static INSTALL: std::sync::Mutex<Option<CefStatus>> = std::sync::Mutex::new(None);

    fn lock_install() -> std::sync::MutexGuard<'static, Option<CefStatus>> {
        INSTALL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What [`install`] is doing, if it was called in this process.
    pub fn status() -> Option<CefStatus> {
        lock_install().clone()
    }

    /// Downloads CEF into `cef_dir` on a new thread, unless a download is already running.
    /// `ctx` is repainted while it runs, and when it is done.
    pub fn install(ctx: &egui::Context, cef_dir: Option<PathBuf>) {
        {
            let mut install = lock_install();
            if matches!(
                *install,
                Some(CefStatus::Installing { .. } | CefStatus::Installed)
            ) {
                return;
            }
            *install = Some(CefStatus::Installing {
                downloaded: 0,
                total: None,
            });
        }
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("egui_webview_cef_install".to_owned())
            .spawn({
                let ctx = ctx.clone();
                move || {
                    let result = run(cef_dir.as_deref(), &ctx);
                    if let Err(err) = &result {
                        log::warn!("egui_webview: failed to install CEF: {err}");
                    }
                    *lock_install() = Some(match result {
                        Ok(()) => CefStatus::Installed,
                        Err(err) => CefStatus::InstallFailed(err),
                    });
                    ctx.request_repaint();
                }
            });
        if let Err(err) = spawned {
            *lock_install() = Some(CefStatus::InstallFailed(err.to_string()));
            ctx.request_repaint();
        }
    }

    /// Installs CEF into its folder in `cef_dir`, unless it is already there.
    fn run(cef_dir: Option<&Path>, ctx: &egui::Context) -> Result<(), String> {
        let cef_dir = cef_dir
            .ok_or("set `CefInitOptions::cef_dir` to the folder that CEF should be installed in")?;
        if ARCHIVE.is_empty() || ARCHIVE_SHA1.is_empty() {
            return Err("this build doesn't know which CEF archive to download".to_owned());
        }
        let dest = versioned_dir(cef_dir).ok_or("the CEF archive has no version in its name")?;
        let parent = dest.parent().ok_or("the CEF folder has no parent")?;
        std::fs::create_dir_all(parent).map_err(|err| io_error("create", parent, &err))?;

        // Only one process installs at a time. A second one waits here, and then finds CEF
        // installed.
        let lock_path = parent.join("install.lock");
        let lock = File::create(&lock_path).map_err(|err| io_error("create", &lock_path, &err))?;
        lock.lock()
            .map_err(|err| io_error("lock", &lock_path, &err))?;
        if dest.join(LIBRARY).exists() {
            return Ok(());
        }

        // Unpack next to the final folder and rename at the end. So a folder with CEF's
        // library in it is always a complete install.
        let partial = dest.with_extension("partial");
        remove_dir(&partial)?;
        if let Err(err) = download_and_unpack(&partial, ctx) {
            // Don't leave up to 1.5 GB of a failed download behind.
            _ = remove_dir(&partial);
            return Err(err);
        }
        let archive_json = format!(
            "{{\n  \"type\": \"minimal\",\n  \"name\": \"{ARCHIVE}\",\n  \"sha1\": \"{ARCHIVE_SHA1}\"\n}}"
        );
        let archive_json_path = partial.join("archive.json");
        std::fs::write(&archive_json_path, archive_json)
            .map_err(|err| io_error("write", &archive_json_path, &err))?;
        remove_dir(&dest)?;
        std::fs::rename(&partial, &dest).map_err(|err| io_error("rename", &partial, &err))
    }

    /// Removes `dir` and what is in it, if it exists.
    fn remove_dir(dir: &Path) -> Result<(), String> {
        match std::fs::remove_dir_all(dir) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
                Err(io_error("remove", dir, &err))
            }
            _ => Ok(()),
        }
    }

    fn io_error(action: &str, path: &Path, err: &std::io::Error) -> String {
        format!("failed to {action} {}: {err}", path.display())
    }

    /// Downloads the CEF archive and unpacks the files that CEF needs at run time into `dest`.
    ///
    /// The archive is unpacked while it downloads. Its SHA-1 is checked at the end, before
    /// anything uses the files.
    fn download_and_unpack(dest: &Path, ctx: &egui::Context) -> Result<(), String> {
        let base = std::env::var("CEF_DOWNLOAD_URL").unwrap_or_else(|_| DOWNLOAD_URL.to_owned());
        // A `+` in a URL path is read as a space by some servers.
        let url = format!("{base}/{}", ARCHIVE.replace('+', "%2B"));
        let response = ureq::get(&url)
            .call()
            .map_err(|err| format!("failed to download CEF from {base}: {err}"))?;
        let total = response.body().content_length();
        let mut download = Download {
            body: response.into_body().into_reader(),
            sha1: sha1_smol::Sha1::new(),
            downloaded: 0,
            total,
            reported: 0,
            ctx,
        };

        let unpacked = unpack(&mut download, dest);
        // Read what the archive reader left, so that the hash covers the whole file.
        let rest = std::io::copy(&mut download, &mut std::io::sink());
        unpacked?;
        rest.map_err(|err| format!("failed to download CEF from {base}: {err}"))?;

        if total.is_some_and(|total| total != download.downloaded) {
            return Err(format!(
                "the CEF download from {base} stopped after {} of {} bytes",
                download.downloaded,
                total.unwrap_or_default()
            ));
        }
        let sha1 = download.sha1.digest().to_string();
        if sha1 != ARCHIVE_SHA1 {
            return Err(format!(
                "the CEF download from {base} is corrupt: its SHA-1 is {sha1}, expected \
                 {ARCHIVE_SHA1}"
            ));
        }
        Ok(())
    }

    /// Unpacks the files that CEF needs at run time from the `.tar.bz2` in `archive` into
    /// `dest`.
    fn unpack(archive: &mut impl Read, dest: &Path) -> Result<(), String> {
        let unpack_error = |err: std::io::Error| format!("failed to unpack CEF: {err}");
        let decoder = bzip2::read::BzDecoder::new(BufReader::new(archive));
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries().map_err(unpack_error)? {
            let mut entry = entry.map_err(unpack_error)?;
            let path = entry.path().map_err(unpack_error)?;
            let Some(relative) = runtime_path(&path) else {
                continue;
            };
            let path = dest.join(relative);
            let entry_type = entry.header().entry_type();
            if entry_type.is_dir() {
                std::fs::create_dir_all(&path).map_err(|err| io_error("create", &path, &err))?;
            } else if entry_type.is_file() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|err| io_error("create", parent, &err))?;
                }
                // This also sets the file's mode, which CEF's libraries and `chrome-sandbox`
                // need.
                entry
                    .unpack(&path)
                    .map_err(|err| io_error("unpack", &path, &err))?;
            }
            // Other kinds of entries, such as links, could point out of `dest`. CEF's
            // archives for Linux and Windows have none.
        }
        Ok(())
    }

    /// Where the archive's file `path` goes in the folder with CEF's files. `None` for a file
    /// that CEF doesn't need at run time, such as a header.
    ///
    /// The archive has one top folder. The files in its `Release` and `Resources` folders go
    /// into one flat folder, as the `cef` crate does at build time.
    fn runtime_path(path: &Path) -> Option<PathBuf> {
        let mut components = path.components();
        let _top = components.next()?;
        let Component::Normal(folder) = components.next()? else {
            return None;
        };
        let is_runtime_folder =
            folder == "Release" || (folder == "Resources" && !cfg!(target_os = "macos"));
        let relative = components.as_path();
        // A path with `..` in it could leave the folder.
        let is_plain = relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
        (is_runtime_folder && is_plain && !relative.as_os_str().is_empty())
            .then(|| relative.to_owned())
    }

    /// The download's body. It hashes what is read from it, and reports the progress.
    struct Download<'a, R> {
        body: R,
        sha1: sha1_smol::Sha1,
        downloaded: u64,
        total: Option<u64>,

        /// The value of `downloaded` when the progress was last reported.
        reported: u64,
        ctx: &'a egui::Context,
    }

    impl<R: Read> Read for Download<'_, R> {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = self.body.read(buffer)?;
            self.sha1.update(buffer.get(..read).unwrap_or_default());
            self.downloaded += read as u64;
            // Often enough for a smooth progress bar, without a repaint for every packet.
            if self.reported + (1 << 20) <= self.downloaded || read == 0 {
                self.reported = self.downloaded;
                *super::download::lock_install() = Some(CefStatus::Installing {
                    downloaded: self.downloaded,
                    total: self.total,
                });
                self.ctx.request_repaint();
            }
            Ok(read)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn runtime_files_go_into_one_flat_folder() {
            let top = "cef_binary_1.2.3+gabc+chromium-1.0.0.0_linux64_minimal";
            let path = |path: &str| runtime_path(&Path::new(top).join(path));
            assert_eq!(path("Release/libcef.so"), Some("libcef.so".into()));
            assert_eq!(
                path("Resources/locales/en-US.pak"),
                (!cfg!(target_os = "macos")).then(|| "locales/en-US.pak".into())
            );
            assert_eq!(path("include/cef_app.h"), None);
            assert_eq!(path("CMakeLists.txt"), None);
            assert_eq!(path("Release"), None);
            assert_eq!(path("Release/../../evil"), None);
        }

        /// Downloads a few hundred MB, so it only runs when asked:
        /// `cargo test -p egui_webview --features cef-download -- --ignored`
        #[test]
        #[ignore = "downloads CEF"]
        fn install_puts_cef_where_find_looks() {
            let dir = tempfile::tempdir().expect("no temporary folder");
            run(Some(dir.path()), &egui::Context::default()).expect("the install failed");

            let options = crate::CefInitOptions {
                cef_dir: Some(dir.path().to_owned()),
                ..Default::default()
            };
            // An executable in a folder without CEF, so that only `cef_dir` can have it.
            let found = super::super::find(&options, &dir.path().join("app"));
            assert_eq!(
                found.ok(),
                versioned_dir(dir.path()).and_then(|dir| std::path::absolute(dir).ok())
            );
        }
    }
}
