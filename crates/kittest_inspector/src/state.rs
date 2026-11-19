//! App state and command dispatch.
//!
//! `AppState` owns all mutable state that persists across frames. UI code never mutates it
//! directly — instead it reads through [`AppStateRef`] (a read-only view with a sender for
//! dispatch) and sends [`Command`]s. `AppState::handle` is the single place state transitions
//! happen; `AppState::update` clears per-frame flags at the start of each frame.

use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::sync::mpsc;

use eframe::egui::Context;
use egui::accesskit::NodeId;
use egui_kittest::inspector_api::Frame;

/// UI → worker message: "you may send `Continue` to the harness now". Carries any egui events
/// captured in Control mode that the harness should queue.
pub type ReleaseTx = mpsc::Sender<Vec<egui::Event>>;
pub type ReleaseRx = mpsc::Receiver<Vec<egui::Event>>;

/// Internal worker → UI message. Translated into commands by [`crate::app::InspectorApp`].
pub enum WorkerEvent {
    Frame(Box<Frame>),
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
}

/// Fast-forward state for the ⏭ Next button.
#[derive(Debug, Clone, Copy)]
pub enum SkipState {
    Inactive,
    /// Auto-release every incoming frame until `call_site_line` differs from this value.
    UntilNewCallLine(Option<u32>),
}

impl SkipState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::UntilNewCallLine(_))
    }
}

/// All the ways persistent state can change. Routed through the app's command inbox.
pub enum Command {
    // --- Playback control ---
    Play,
    Pause,
    Step,
    NextCall,
    /// End-of-frame auto-release: release the harness if it's waiting. No-op otherwise.
    AutoRelease,

    // --- History navigation ---
    SetViewIndex(usize),
    GoLive,

    // --- Selection ---
    SetSelectedNode(Option<NodeId>),

    // --- Control mode ---
    SetControlEnabled(bool),

    // --- Worker events ---
    WorkerFrame(Box<Frame>),
    WorkerDisconnected,

    // --- Cached image placement (written by central panel after rendering) ---
    SetImageRect {
        rect: egui::Rect,
        scale: f32,
    },

    // --- GIF ---
    StartCopyAsGif,
}

/// Owns every piece of state that survives across frames. Mutated only via [`Self::handle`].
pub struct AppState {
    pub play_state: PlayState,
    /// True when the worker is blocked waiting for a release.
    pub worker_waiting: bool,
    /// Every frame the harness has ever sent, in order. Supports back/forward replay.
    pub history: Vec<Frame>,
    /// Index into `history` of the currently-displayed frame.
    pub view_index: usize,
    pub connected: bool,
    /// Last clicked widget (sticky across frames).
    pub selected_node: Option<NodeId>,
    /// When on, pointer + keyboard events are forwarded to the harness.
    pub control_enabled: bool,
    /// Events accumulated since the last release; drained when we send Continue.
    pub queued_events: Vec<egui::Event>,
    /// Set when the viewed frame changes; the Source section consumes it to scroll once.
    pub scroll_pending: bool,
    /// While `UntilNewCallLine`, auto-release every incoming frame until we see one with a
    /// different `call_site_line` — i.e. until the test moves past the current runner call.
    pub skip: SkipState,
    /// Screen rect of the rendered image from the previous frame. We hit-test against this
    /// at the start of the next `ui()` (before panels render) so the details tree can see
    /// `hovered_node` in the same frame as the image highlight.
    pub last_image_rect: Option<egui::Rect>,
    /// Display-pixel-per-physical-pixel ratio from the previous frame.
    pub last_image_scale: f32,
    /// Transient status line (e.g. "Copied to /tmp/...") shown next to the Copy-GIF button.
    pub status_message: Option<String>,

    release_tx: ReleaseTx,
}

impl AppState {
    pub fn new(release_tx: ReleaseTx) -> Self {
        Self {
            play_state: PlayState::Paused,
            worker_waiting: false,
            history: Vec::new(),
            view_index: 0,
            connected: true,
            selected_node: None,
            control_enabled: false,
            queued_events: Vec::new(),
            scroll_pending: false,
            skip: SkipState::Inactive,
            last_image_rect: None,
            last_image_scale: 1.0,
            status_message: None,
            release_tx,
        }
    }

    /// The frame currently being displayed. `None` only before the first frame ever arrives.
    pub fn view_frame(&self) -> Option<&Frame> {
        self.history.get(self.view_index)
    }

    /// True when `view_index` points at the most recent frame (so new arrivals keep scrolling
    /// the view forward).
    pub fn is_live_view(&self) -> bool {
        !self.history.is_empty() && self.view_index + 1 == self.history.len()
    }

    /// Per-frame tick, called at the very top of `ui()` *before* commands are drained. Use
    /// this for state that should reset each frame unless a command re-asserts it.
    pub fn update(&mut self, _ctx: &Context) {
        self.scroll_pending = false;
    }

    /// Apply a single command. The only entry point for state transitions.
    pub fn handle(&mut self, _ctx: &Context, command: Command) {
        match command {
            Command::Play => {
                self.play_state = PlayState::Playing;
                self.send_release();
            }
            Command::Pause => {
                self.play_state = PlayState::Paused;
            }
            Command::Step => {
                self.send_release();
            }
            Command::NextCall => {
                // "From" is the *live* frame's call_site — the harness is blocked there, not
                // at wherever the user is currently browsing in history.
                let current_line = self
                    .history
                    .last()
                    .and_then(|f| f.source.as_ref())
                    .and_then(|s| s.call_site_line);
                self.skip = SkipState::UntilNewCallLine(current_line);
                self.send_release();
            }
            Command::AutoRelease => {
                let auto = if self.skip.is_active() {
                    true
                } else if self.control_enabled {
                    !self.queued_events.is_empty()
                } else {
                    self.play_state == PlayState::Playing
                };
                if auto {
                    self.send_release();
                }
            }

            Command::SetViewIndex(idx) => {
                self.set_view_index(idx);
            }
            Command::GoLive => {
                let total = self.history.len();
                self.set_view_index(total.saturating_sub(1));
            }

            Command::SetSelectedNode(n) => {
                self.selected_node = n;
            }

            Command::SetControlEnabled(v) => {
                if self.control_enabled && !v {
                    self.queued_events.clear();
                }
                self.control_enabled = v;
            }

            Command::WorkerFrame(frame) => {
                let new_call_line = frame.source.as_ref().and_then(|s| s.call_site_line);
                let was_live = self.is_live_view() || self.history.is_empty();
                self.history.push(*frame);
                if was_live {
                    self.view_index = self.history.len() - 1;
                }
                self.worker_waiting = true;

                // If we're fast-forwarding to the next `run()` call, stop once the call_site
                // line differs from the one we started from.
                let still_skipping = matches!(
                    self.skip,
                    SkipState::UntilNewCallLine(from) if new_call_line == from
                );
                if still_skipping {
                    // Don't auto-scroll / flash for in-between frames we're about to blow
                    // past; the user will see the first settled frame at the new call.
                } else {
                    self.skip = SkipState::Inactive;
                    if was_live {
                        self.scroll_pending = true;
                    }
                }
            }
            Command::WorkerDisconnected => {
                self.connected = false;
                self.worker_waiting = false;
                self.skip = SkipState::Inactive;
                // Nothing to drive any more.
                self.control_enabled = false;
                self.queued_events.clear();
            }

            Command::SetImageRect { rect, scale } => {
                self.last_image_rect = Some(rect);
                self.last_image_scale = scale;
            }

            Command::StartCopyAsGif => {
                self.start_copy_as_gif();
            }
        }
    }

    fn set_view_index(&mut self, idx: usize) {
        let idx = idx.min(self.history.len().saturating_sub(1));
        if idx != self.view_index {
            self.view_index = idx;
            self.scroll_pending = true;
        }
    }

    fn send_release(&mut self) {
        if !self.worker_waiting {
            return;
        }
        let events = std::mem::take(&mut self.queued_events);
        if self.release_tx.send(events).is_ok() {
            self.worker_waiting = false;
        }
    }

    fn start_copy_as_gif(&mut self) {
        crate::log_diag("Copy as GIF clicked");
        // Run the copy on a detached worker so a slow encode doesn't stall the UI.
        let history = self.history.clone();
        let _ignored = std::thread::Builder::new()
            .name("kittest_inspector_copy_gif".into())
            .spawn(
                move || match crate::ui::copy_history_as_gif(&history, 10.0) {
                    Ok(path) => {
                        crate::log_diag(&format!("Copied GIF to clipboard: {}", path.display()));
                    }
                    Err(err) => crate::log_diag(&format!("Failed to copy GIF: {err}")),
                },
            );
        self.status_message = Some("Encoding + copying GIF on background thread — see log".into());
    }
}

/// Read-only view of [`AppState`] that UI code receives. Sends [`Command`]s via `.send(...)`.
///
/// Derives like `view_frame` are computed once by [`AppState::reference`] so every panel in the
/// same frame sees identical values. Per-frame ephemeral state (`hovered_node`, in-frame event
/// capture) lives in interior-mutable fields on this ref — the state itself stays read-only.
pub struct AppStateRef<'a> {
    pub state: &'a AppState,
    pub tx: mpsc::Sender<Command>,
    pub texture: Option<&'a egui::TextureHandle>,
    pub view_frame: Option<&'a Frame>,
    /// Hover resolved this frame. Seeded from `hit_test_pointer` (image hover based on the
    /// cursor and the cached image rect), and may be overwritten during tree rendering when
    /// the user hovers a tree row.
    pub hovered_node: Cell<Option<NodeId>>,
    /// Events captured in Control mode during this frame. Transferred to `state.queued_events`
    /// after UI rendering so end-of-frame auto-release sees them synchronously.
    pub captured_events: RefCell<Vec<egui::Event>>,
}

impl<'a> AppState {
    pub fn reference(
        &'a self,
        texture: Option<&'a egui::TextureHandle>,
        tx: mpsc::Sender<Command>,
        initial_hover: Option<NodeId>,
    ) -> AppStateRef<'a> {
        AppStateRef {
            state: self,
            tx,
            texture,
            view_frame: self.view_frame(),
            hovered_node: Cell::new(initial_hover),
            captured_events: RefCell::new(Vec::new()),
        }
    }
}

impl AppStateRef<'_> {
    pub fn send(&self, cmd: impl Into<Command>) {
        self.tx.send(cmd.into()).ok();
    }
}

impl Deref for AppStateRef<'_> {
    type Target = AppState;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}
