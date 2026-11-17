//! What the [`crate::WebView`] widget and the [`crate::WebViews`] registry trade.
//!
//! The widget can't hold a page, because pages aren't `Send`. So the widget writes what it
//! wants into the mailbox during the pass, and the registry acts on it after the pass. The
//! registry writes what the page did, and the widget reads that in the next pass.
//!
//! Everything in here is plain data, so it can live in [`egui::Context::data`].

use std::sync::Arc;

use egui::mutex::Mutex;
use egui::{Context, Id, ViewportId};

use crate::instance::FocusChange;
use crate::{PageWidget, WebViewEvent, WebViewSource};

/// The number of the root viewport's current pass (see [`Context::cumulative_pass_nr`]).
///
/// Webviews are only supported in the root viewport, so its passes are the only clock.
pub(crate) fn now(ctx: &Context) -> u64 {
    ctx.cumulative_pass_nr_for(ViewportId::ROOT)
}

/// Whether `pass` is the root viewport's latest pass. Only call this between passes.
pub(crate) fn is_latest(ctx: &Context, pass: u64) -> bool {
    pass + 1 == now(ctx)
}

/// The mailbox itself. The widget and the registry each hold a clone.
#[derive(Clone)]
pub(crate) struct Mailbox(Arc<Mutex<Inner>>);

pub(crate) struct Inner {
    /// The widget for a page that doesn't exist yet. With it, an embedded page gets its rect
    /// in the first pass, so the registry can place it in its first frame.
    pub fallback: Option<Arc<dyn PageWidget>>,

    /// One slot per webview id. A `Vec`, because there are few and they are visited in order.
    pub webviews: Vec<(Id, WebViewState)>,
}

impl Inner {
    /// The webview state of `id`. Creates it if it doesn't exist.
    pub fn get_or_create_state(&mut self, id: Id) -> &mut WebViewState {
        let index = self.webviews.iter().position(|(slot_id, _)| *slot_id == id);
        let index = index.unwrap_or_else(|| {
            self.webviews.push((id, WebViewState::default()));
            self.webviews.len() - 1
        });
        &mut self.webviews[index].1
    }

    /// The webview state of `id`, if it exists.
    pub fn get_state(&mut self, id: Id) -> Option<&mut WebViewState> {
        self.webviews
            .iter_mut()
            .find(|(slot_id, _)| *slot_id == id)
            .map(|(_, slot)| slot)
    }
}

impl Mailbox {
    pub fn new(fallback: Option<Arc<dyn PageWidget>>) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            fallback,
            webviews: Vec::new(),
        })))
    }

    fn key() -> Id {
        Id::unique("egui_webview::Mailbox")
    }

    /// The mailbox that a [`crate::WebViews`] put into `ctx`, if there is one.
    pub fn of(ctx: &Context) -> Option<Self> {
        ctx.data(|data| data.get_temp(Self::key()))
    }

    /// Puts this mailbox into `ctx`, if it isn't there already.
    pub fn install(&self, ctx: &Context) {
        let installed = Self::of(ctx).is_some_and(|other| Arc::ptr_eq(&other.0, &self.0));
        if !installed {
            ctx.data_mut(|data| data.insert_temp(Self::key(), self.clone()));
        }
    }

    /// Locks the mailbox for `f`. Don't run egui or backend code in `f`.
    pub fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        f(&mut self.0.lock())
    }

    /// Locks the mailbox for `f`, running `f` with the webview state of `id`, if it exists.
    pub fn with_view_state<R>(&self, id: Id, f: impl FnOnce(&mut WebViewState) -> R) -> Option<R> {
        self.with(|inner| {
            if let Some(state) = inner.get_state(id) {
                return Some(f(state));
            }
            None
        })
    }
}

/// One webview's data. Only [`crate::WebView::show`] creates a slot, so every slot stands for a
/// page that some code wants to see.
#[derive(Default)]
pub(crate) struct WebViewState {
    pub source: Option<Arc<WebViewSource>>,

    pub alive_pass: Option<u64>,
    pub visible: bool,

    pub page_command_queue: Vec<Command>,
    pub events: Vec<WebViewEvent>,

    pub ui: UiState,

    pub needs_repaint: bool,
}

#[derive(Default)]
pub struct UiState {
    pub widget: Option<WidgetResult>,
    /// Any pending focus change the widget needs to know about next pass.
    pub pending_focus_change: Option<FocusChange>,
}

type WidgetResult = Result<Arc<dyn PageWidget>, String>;

impl WebViewState {
    pub fn mark_shown(&mut self, pass: u64) {
        self.mark_alive(pass);
        self.visible = true;
    }
    pub fn mark_alive(&mut self, pass: u64) {
        self.alive_pass = Some(pass);
    }

    pub fn extend_events(&mut self, events: impl IntoIterator<Item = WebViewEvent>) {
        let len = self.events.len();
        self.events.extend(events);
        self.needs_repaint |= len != self.events.len();
    }

    pub fn set_page_widget(&mut self, page_widget: WidgetResult) {
        self.ui.widget = Some(page_widget);
        self.needs_repaint = true;
    }

    pub fn push_command(&mut self, command: Command) {
        self.page_command_queue.push(command);
    }
}

pub(crate) enum Command {
    Load(WebViewSource),
    Reload,
    Back,
    Forward,
    EvaluateScript(String),
    ChangeFocus(FocusChange),
}
