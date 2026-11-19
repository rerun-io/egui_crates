//! Eframe app that displays frames + accesskit trees streamed from an `egui_kittest` harness,
//! and lets the user pause / resume / single-step the test and inspect individual widgets.
//!
//! The binary in `src/main.rs` is the default entry point: it wires up stdin/stdout I/O, a
//! single-instance lock, and runs the eframe app. This crate also exposes [`InspectorApp`]
//! and the worker-channel types as a library so integration tests can launch the same app
//! under `egui_kittest` and feed synthetic frames through the channels.
//!
//! Architecture follows kitdiff: `AppState` owns all persistent state; UI code in
//! `ui` receives a read-only `AppStateRef` and dispatches `Command`s via
//! `.send(...)`. The single place state actually mutates is `AppState::handle`.

mod app;
mod gif;
mod state;
mod ui;

pub use app::InspectorApp;
pub use state::{ReleaseRx, ReleaseTx, WorkerEvent};

/// Append a diagnostic line to `{temp}/kittest_inspector.log`.
///
/// We do NOT write to stderr — when the harness's captured stderr pipe closes mid-run,
/// `eprintln!` panics with "failed printing to stderr: Broken pipe" and kills the window.
pub fn log_diag(msg: &str) {
    use std::io::Write as _;

    #[expect(clippy::disallowed_methods)]
    let path = std::env::temp_dir().join("kittest_inspector.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let _ignored = writeln!(f, "[{ts}] {msg}");
    }
}
