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

use crate::instance::{FocusChange, PageFocus};
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
    pub slots: Vec<(Id, Slot)>,
}

impl Inner {
    /// The slot of `id`. Creates it if it doesn't exist.
    pub fn slot(&mut self, id: Id) -> &mut Slot {
        let index = self.slots.iter().position(|(slot_id, _)| *slot_id == id);
        let index = index.unwrap_or_else(|| {
            self.slots.push((id, Slot::default()));
            self.slots.len() - 1
        });
        &mut self.slots[index].1
    }

    /// The slot of `id`, if it exists.
    pub fn existing_slot(&mut self, id: Id) -> Option<&mut Slot> {
        self.slots
            .iter_mut()
            .find(|(slot_id, _)| *slot_id == id)
            .map(|(_, slot)| slot)
    }
}

impl Mailbox {
    pub fn new(fallback: Option<Arc<dyn PageWidget>>) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            fallback,
            slots: Vec::new(),
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
}

/// One webview's data. Only [`crate::WebView::show`] creates a slot, so every slot stands for a
/// page that some code wants to see.
#[derive(Default)]
pub(crate) struct Slot {
    pub to_page: ToPage,
    pub to_widget: ToWidget,
}

/// The widget writes this in the pass. The registry reads it after the pass.
#[derive(Default)]
pub(crate) struct ToPage {
    // These pile up until the registry takes them. A pass with nothing to say leaves them alone.
    // A discarded pass is followed by one with no input, and that one must not erase them.
    /// From [`crate::WebView::reload`] and friends. Oldest first.
    pub commands: Vec<Command>,

    /// What happened to the widget's egui focus. The latest change wins.
    pub focus: Option<FocusChange>,

    // These describe the latest pass. Every pass that shows the widget replaces them.
    /// What the builder was told to show. It is shared, so a long HTML string isn't copied in
    /// every pass.
    pub source: Option<Arc<WebViewSource>>,

    /// The pass that last showed the widget.
    pub shown: Option<u64>,

    /// Whether the widget had egui's focus at the end of that pass.
    pub focused: bool,

    /// The pass that last asked for the page to stay. [`crate::WebView::show`] sets it, and so
    /// does [`crate::WebView::keep_alive`].
    pub alive: Option<u64>,
}

impl ToPage {
    /// Takes what piled up, and copies the rest.
    pub fn take(&mut self) -> Self {
        Self {
            commands: std::mem::take(&mut self.commands),
            focus: self.focus.take(),
            source: self.source.clone(),
            shown: self.shown,
            focused: self.focused,
            alive: self.alive,
        }
    }
}

pub(crate) enum Command {
    Load(WebViewSource),
    Reload,
    Back,
    Forward,
    EvaluateScript(String),
}

/// The registry writes this after the pass. The widget reads it in the next pass.
#[derive(Default)]
pub(crate) struct ToWidget {
    /// `None` until the page exists.
    pub widget: Option<Arc<dyn PageWidget>>,

    /// What happened in the page. The widget takes them.
    pub events: Vec<WebViewEvent>,

    /// The page took the focus, or Tab left it. The widget takes it.
    pub focus: Option<PageFocus>,

    /// Why the page couldn't be created. The widget shows this text in place of the page.
    pub failed: Option<String>,
}

impl ToWidget {
    /// How many events are kept for a page that is kept alive but not shown.
    const MAX_WAITING_EVENTS: usize = 1024;

    /// Adds what the registry has to tell the widget after this frame. Returns whether the
    /// widget has something new to read.
    ///
    /// In `news`, `widget` and `focus` are only set when they changed. `failed` is the state
    /// as it is now. `shown` is whether the widget was shown in the latest pass.
    pub fn merge(&mut self, news: Self, shown: bool) -> bool {
        let mut changed = !news.events.is_empty();
        self.events.extend(news.events);
        let waiting = self.events.len();
        if Self::MAX_WAITING_EVENTS < waiting {
            self.events.drain(..waiting - Self::MAX_WAITING_EVENTS);
        }

        if let Some(widget) = news.widget {
            self.widget = Some(widget);
            changed = true;
        }
        if self.failed != news.failed {
            self.failed = news.failed;
            changed = true;
        }

        if !shown {
            // Nobody will act on it, and it would be stale by the time the widget is back.
            self.focus = None;
        } else if news.focus.is_some() {
            self.focus = news.focus;
            changed = true;
        }
        changed
    }
}
