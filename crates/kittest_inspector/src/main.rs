//! Binary entry point for the kittest inspector.
//!
//! The app itself lives in [`kittest_inspector`]; this file only wires up stdin/stdout I/O,
//! a cross-process single-instance guard, and the eframe event loop.
//!
//! Communication with the harness is over stdin/stdout: the harness pipes [`HarnessMessage`]s
//! into our stdin and reads [`InspectorReply`]s from our stdout. All logging goes to a file.

use std::io::{self, BufReader, BufWriter};
use std::sync::mpsc;
use std::thread;

use egui_kittest::inspector_api::{HarnessMessage, InspectorReply, read_message, write_message};
use kittest_inspector::{InspectorApp, ReleaseRx, WorkerEvent, log_diag};

fn main() -> eframe::Result<()> {
    // Install a panic hook that writes to our own log file (not the inherited — and
    // potentially captured — stderr of the harness). Whatever the main thread or a spawned
    // worker does, we always get a breadcrumb on disk.
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_diag(&format!("PANIC: {info}"));
        default(info);
    }));

    // Cross-process single-instance guard. If another inspector is already running, block
    // here until that window closes. Held for the lifetime of `_lock`; the OS releases the
    // flock when the file descriptor is dropped on exit.
    let _lock = acquire_single_instance_lock();

    let (worker_tx, worker_rx) = mpsc::channel::<WorkerEvent>();
    let (release_tx, release_rx) = mpsc::channel::<Vec<egui::Event>>();

    thread::Builder::new()
        .name("kittest_inspector_io".into())
        .spawn(move || run_io(&worker_tx, &release_rx))
        .expect("spawn io thread");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("kittest inspector")
            .with_inner_size([1100.0, 750.0]),
        ..Default::default()
    };

    eframe::run_native(
        "kittest inspector",
        options,
        Box::new(|cc| Ok(Box::new(InspectorApp::new(cc, worker_rx, release_tx)))),
    )
}

/// Read frames from stdin, forward to UI, wait for a release, then write Continue to stdout.
fn run_io(ui_tx: &mpsc::Sender<WorkerEvent>, release_rx: &ReleaseRx) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    loop {
        match read_message::<_, HarnessMessage>(&mut reader) {
            Ok(HarnessMessage::Frame(frame)) => {
                if ui_tx.send(WorkerEvent::Frame(frame)).is_err() {
                    return;
                }
                let Ok(events) = release_rx.recv() else {
                    return;
                };
                if let Err(err) = write_message(&mut writer, &InspectorReply::Continue { events }) {
                    log_diag(&format!("write failed: {err}"));
                    return;
                }
            }
            Ok(HarnessMessage::Goodbye) => {
                let _ignored = ui_tx.send(WorkerEvent::Disconnected);
                return;
            }
            Err(err) => {
                if err.kind() != io::ErrorKind::UnexpectedEof {
                    log_diag(&format!("read failed: {err}"));
                }
                let _ignored = ui_tx.send(WorkerEvent::Disconnected);
                return;
            }
        }
    }
}

/// Try to acquire a cross-process exclusive lock on a well-known file so that only one
/// inspector window can be open on the machine at a time. Blocks here (before we open any
/// windows or touch stdio beyond this stderr line) if another inspector is already running.
fn acquire_single_instance_lock() -> Option<std::fs::File> {
    // We specifically need a stable, cross-process path here — tempfile's per-process dir
    // can't serve as a system-wide mutex.
    #[expect(clippy::disallowed_methods)]
    let path = std::env::temp_dir().join("kittest_inspector.lock");

    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(f) => f,
        Err(err) => {
            log_diag(&format!(
                "couldn't open lock file {}: {err} (running without single-instance guard)",
                path.display()
            ));
            return None;
        }
    };

    match file.lock() {
        Ok(()) => Some(file),
        Err(err) => {
            log_diag(&format!("failed to acquire lock: {err}"));
            None
        }
    }
}
