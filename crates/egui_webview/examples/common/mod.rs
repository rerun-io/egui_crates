//! Helpers shared by the test examples: builders for synthetic egui input events, and a plugin
//! that injects them into egui's input.

use std::collections::VecDeque;
use std::sync::Arc;

use egui::mutex::Mutex;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};

/// Synthetic events waiting to be injected into egui's input. Each entry holds one pass's events.
pub type Queue = Arc<Mutex<VecDeque<Vec<Event>>>>;

/// A primary-button click at `pos`: a pointer move, a press and a release, in three passes.
pub fn click(pos: Pos2) -> Vec<Vec<Event>> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![
        vec![Event::PointerMoved(pos)],
        vec![button(true)],
        vec![button(false)],
    ]
}

/// A press and a release of `key`, in two passes, as egui-winit reports them. The press comes
/// with an [`Event::Text`] of `text`, if given, in the same pass.
pub fn key_with_text(key: Key, modifiers: Modifiers, text: Option<&str>) -> Vec<Vec<Event>> {
    let event = |pressed| Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    };
    let mut press = vec![event(true)];
    press.extend(text.map(|text| Event::Text(text.to_owned())));
    vec![press, vec![event(false)]]
}

pub fn key(key: Key, modifiers: Modifiers) -> Vec<Vec<Event>> {
    key_with_text(key, modifiers, None)
}

/// An [`egui::Plugin`] that injects the next entry of its [`Queue`] into egui's input every pass.
pub struct Injector(pub Queue);

impl egui::Plugin for Injector {
    fn debug_name(&self) -> &'static str {
        "egui_webview test input"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut RawInput) {
        // `Response::has_focus` is `false` in a window without the OS focus. The test window may
        // open without it (`cef_input_test` opens inactive so it doesn't take the focus from the
        // user, and Windows then reports it unfocused), so always report it as focused.
        input.focused = true;
        if let Some(events) = self.0.lock().pop_front() {
            input.events.extend(events);
        }
    }
}
