//! The `eframe::App` wrapper that owns `AppState` plus the side-channel I/O (worker/release
//! channels, texture upload, hit-testing). Every frame:
//!
//! 1. `state.update(ctx)` clears per-frame flags.
//! 2. Worker events are translated to commands.
//! 3. The command inbox is drained into `state.handle(...)`.
//! 4. The GPU texture is synced to the viewed frame.
//! 5. Pointer hit-testing produces an initial hovered node.
//! 6. An `AppStateRef` is built and UI panels render.
//! 7. End-of-frame: captured events (from Control mode) are promoted into state, and an
//!    `AutoRelease` command is enqueued for next frame.

use std::sync::mpsc;

use egui::accesskit::NodeId;
use egui_kittest::inspector_api::Frame;

use crate::state::{AppState, Command, WorkerEvent};
use crate::ui;

pub struct InspectorApp {
    state: AppState,
    /// Sender side of the command inbox. Cloned into `AppStateRef` for UI dispatch and into
    /// the state itself for async workers (GIF thread) to report back.
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Command>,
    worker_rx: mpsc::Receiver<WorkerEvent>,
    /// `Frame::step` currently uploaded to `current_texture` — used to decide whether the
    /// texture needs regenerating when `view_index` changes.
    textured_step: Option<u64>,
    current_texture: Option<egui::TextureHandle>,
}

impl InspectorApp {
    pub fn new(
        _cc: &eframe::CreationContext<'_>,
        worker_rx: mpsc::Receiver<WorkerEvent>,
        release_tx: mpsc::Sender<Vec<egui::Event>>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let state = AppState::new(release_tx);
        Self {
            state,
            tx,
            rx,
            worker_rx,
            textured_step: None,
            current_texture: None,
        }
    }

    /// Drain worker events and enqueue the matching commands.
    fn pump_worker(&self) {
        while let Ok(event) = self.worker_rx.try_recv() {
            let cmd = match event {
                WorkerEvent::Frame(frame) => Command::WorkerFrame(frame),
                WorkerEvent::Disconnected => Command::WorkerDisconnected,
            };
            let _ignored = self.tx.send(cmd);
        }
    }

    fn drain_inbox(&mut self, ctx: &egui::Context) {
        while let Ok(cmd) = self.rx.try_recv() {
            self.state.handle(ctx, cmd);
        }
    }

    /// (Re-)upload `view_frame()`'s pixels to `current_texture` if the texture is missing or
    /// represents a different step than what we're viewing.
    fn ensure_texture_uploaded(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.state.view_frame() else {
            return;
        };
        if self.textured_step == Some(frame.step) {
            return;
        }
        let size = [frame.width as usize, frame.height as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
        let texture = ctx.load_texture("kittest_inspector_frame", color_image, Default::default());
        self.textured_step = Some(frame.step);
        self.current_texture = Some(texture);
    }

    /// Hit-test the cursor against the cached image rect + the viewed frame's accesskit
    /// bounds. Returns the initial hover — the tree may overwrite it during rendering.
    fn hit_test_pointer(&self, ctx: &egui::Context) -> Option<NodeId> {
        if self.state.control_enabled {
            return None; // In control mode we forward events, we don't inspect on hover.
        }
        let image_rect = self.state.last_image_rect?;
        let frame: &Frame = self.state.view_frame()?;
        let update = frame.accesskit.as_ref()?;
        let pos = ctx.input(|i| i.pointer.hover_pos())?;
        if !image_rect.contains(pos) {
            return None;
        }
        let f = (frame.pixels_per_point * self.state.last_image_scale) as f64;
        let lx = ((pos.x - image_rect.min.x) as f64) / f;
        let ly = ((pos.y - image_rect.min.y) as f64) / f;
        let mut best: Option<(NodeId, f64)> = None;
        for (id, node) in &update.nodes {
            let Some(b) = node.bounds() else { continue };
            if lx >= b.x0 && lx <= b.x1 && ly >= b.y0 && ly <= b.y1 {
                let area = (b.x1 - b.x0).max(0.0) * (b.y1 - b.y0).max(0.0);
                if best.is_none_or(|(_, a)| area < a) {
                    best = Some((*id, area));
                }
            }
        }
        best.map(|(id, _)| id)
    }
}

impl eframe::App for InspectorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 1) Per-frame state tick (clears `scroll_pending` so commands below can re-assert it).
        self.state.update(&ctx);

        // 2) Translate worker events to commands.
        self.pump_worker();

        // 3) Drain command inbox.
        self.drain_inbox(&ctx);

        // 4) Sync GPU texture to the frame we're about to render.
        self.ensure_texture_uploaded(&ctx);

        // 5) Resolve initial hover from the cached image rect.
        let initial_hover = self.hit_test_pointer(&ctx);

        // 6) Build the read-only state reference and render.
        let pending_events;
        {
            let state_ref = self.state.reference(
                self.current_texture.as_ref(),
                self.tx.clone(),
                initial_hover,
            );

            ui::controls_panel(ui, &state_ref);
            ui::details_panel(ui, &state_ref);
            ui::central_panel(ui, &state_ref);

            pending_events = state_ref.captured_events.take();
        }

        // 7) End-of-frame: promote Control-mode events into state synchronously so the
        // AutoRelease command we send next sees them, and enqueue the auto-release decision
        // for next frame (it re-reads play/control/skip state at that point).
        if !pending_events.is_empty() {
            self.state.queued_events.extend(pending_events);
        }
        // AutoRelease is enqueued rather than run synchronously so *all* state mutations stay
        // funneled through `AppState::handle`.
        let _ignored = self.tx.send(Command::AutoRelease);

        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
}
