//! Forwarding egui's pointer, wheel, keyboard, text, IME and clipboard events to a CEF browser.

use cef::{Browser, ImplBrowser as _, ImplBrowserHost as _, ImplFrame, KeyEvent, MouseEvent, sys};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Response, Ui, Vec2};

use super::keys;

/// The invalid range, which CEF's IME methods read as "no replacement range".
///
/// CEF ignores an IME call whose range argument is `None`, so this is passed instead.
const NO_RANGE: cef::Range = cef::Range {
    from: u32::MAX,
    to: u32::MAX,
};

/// The points scrolled per line of an [`egui::MouseWheelUnit::Line`] wheel event.
const POINTS_PER_LINE: f32 = 40.0;

/// The pointer state that forwarding input to one page carries from pass to pass.
#[derive(Default)]
pub struct Input {
    /// The mouse buttons pressed on the page and not released yet.
    ///
    /// While one is down, the page keeps getting pointer events outside its rect, like with a
    /// browser's implicit pointer capture.
    buttons_down: Vec<PointerButton>,
    /// Whether the page was sent a mouse move since the last mouse leave.
    pointer_inside: bool,
    /// The time, position, button and click count of the last press, for counting double and
    /// triple clicks.
    last_click: Option<(f64, Pos2, PointerButton, i32)>,
    /// The fraction of the wheel delta not sent yet, since CEF takes whole points.
    wheel_remainder: Vec2,
}

impl Input {
    /// Forwards this pass's pointer events to the page. If `keys` is true and `response` has
    /// focus, it forwards the keyboard, text, IME and clipboard events too.
    pub fn handle(&mut self, ui: &Ui, response: &Response, browser: &Browser, keys: bool) {
        let Some(host) = browser.host() else {
            return;
        };
        // The page scrolls itself, so an enclosing `ScrollArea` mustn't scroll as well.
        if response.contains_pointer() {
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        let events = ui.input(|i| i.events.clone());
        let mut events = events.into_iter().peekable();
        while let Some(event) = events.next() {
            if !self.pointer(ui, response, &host, &event) && keys && response.has_focus() {
                // egui emits the `Event::Text` that a key press typed right after its
                // `Event::Key`, so the two are sent together.
                let text = match event {
                    Event::Key { pressed: true, .. } => {
                        events.next_if(|e| matches!(e, Event::Text(_)))
                    }
                    _ => None,
                };
                keyboard(browser, &host, event, text);
            }
        }
    }

    /// Forwards `event` to the page if it is a pointer event, and returns whether it was one.
    fn pointer(
        &mut self,
        ui: &Ui,
        response: &Response,
        host: &cef::BrowserHost,
        event: &Event,
    ) -> bool {
        let rect = response.rect;
        // False while a layer above the page, such as a window or a popup, covers the pointer.
        let hovered = response.contains_pointer();
        let (time, hover_pos, modifiers) =
            ui.input(|i| (i.time, i.pointer.hover_pos(), i.modifiers));
        let mouse = |pos: Pos2, modifiers: Modifiers, buttons: &[PointerButton]| {
            let pos = pos - rect.min;
            MouseEvent {
                x: super::round(pos.x),
                y: super::round(pos.y),
                modifiers: event_flags(modifiers) | button_flags(buttons),
            }
        };
        match *event {
            Event::PointerMoved(pos) => {
                if hovered || !self.buttons_down.is_empty() {
                    let event = mouse(pos, modifiers, &self.buttons_down);
                    host.send_mouse_move_event(Some(&event), 0);
                    self.pointer_inside = true;
                } else if std::mem::take(&mut self.pointer_inside) {
                    host.send_mouse_move_event(Some(&mouse(pos, modifiers, &[])), 1);
                }
            }
            Event::PointerGone => {
                if std::mem::take(&mut self.pointer_inside) {
                    let event = mouse(hover_pos.unwrap_or(rect.min), modifiers, &[]);
                    host.send_mouse_move_event(Some(&event), 1);
                }
            }
            Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } => {
                let Some(cef_button) = mouse_button(button) else {
                    return true;
                };
                if pressed && hovered {
                    response.request_focus();
                    let count = self.click_count(time, pos, button);
                    self.buttons_down.push(button);
                    let event = mouse(pos, modifiers, &self.buttons_down);
                    host.send_mouse_click_event(Some(&event), cef_button, 0, count);
                } else if !pressed && self.buttons_down.contains(&button) {
                    self.buttons_down.retain(|b| *b != button);
                    let count = self.last_click.map_or(1, |(.., count)| count);
                    let event = mouse(pos, modifiers, &self.buttons_down);
                    host.send_mouse_click_event(Some(&event), cef_button, 1, count);
                }
            }
            Event::MouseWheel {
                unit,
                delta,
                modifiers,
                ..
            } => {
                if hovered {
                    let delta = match unit {
                        egui::MouseWheelUnit::Point => delta,
                        egui::MouseWheelUnit::Line => delta * POINTS_PER_LINE,
                        egui::MouseWheelUnit::Page => delta * rect.height(),
                    };
                    let event = mouse(hover_pos.unwrap_or(rect.min), modifiers, &[]);
                    let delta = delta + self.wheel_remainder;
                    let (x, y) = (super::round(delta.x), super::round(delta.y));
                    #[expect(clippy::cast_precision_loss)]
                    let sent = egui::vec2(x as f32, y as f32);
                    self.wheel_remainder = delta - sent;
                    host.send_mouse_wheel_event(Some(&event), x, y);
                }
            }
            _ => return false,
        }
        true
    }

    /// The click count of a press, which is 2 for the second click of a double click.
    ///
    /// A windowless browser doesn't count clicks, so this counts them like the OS would.
    fn click_count(&mut self, time: f64, pos: Pos2, button: PointerButton) -> i32 {
        const MAX_DELAY: f64 = 0.5;
        const MAX_DISTANCE: f32 = 4.0;
        let count = match self.last_click {
            Some((last_time, last_pos, last_button, count))
                if last_button == button
                    && time - last_time < MAX_DELAY
                    && last_pos.distance(pos) < MAX_DISTANCE =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_click = Some((time, pos, button, count));
        count
    }
}

/// Forwards a keyboard, text, IME or clipboard event to the focused page.
///
/// `text` is the [`Event::Text`] that directly followed a key press, if any.
fn keyboard(browser: &Browser, host: &cef::BrowserHost, event: Event, text: Option<Event>) {
    match event {
        Event::Key {
            key,
            physical_key,
            pressed,
            repeat,
            modifiers,
        } => {
            let text = match text {
                Some(Event::Text(text)) => text,
                _ => String::new(),
            };
            let key = KeyPress {
                key,
                physical_key: physical_key.unwrap_or(key),
                repeat,
                modifiers,
            };
            if !(pressed && mac_command(&key, browser)) {
                key.send(host, pressed, &text);
            }
        }
        Event::Text(text) => {
            let modifiers = Modifiers::NONE;
            for c in text.encode_utf16() {
                send_char(host, c, 0, modifiers);
            }
        }
        Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
            let end = u32::try_from(text.encode_utf16().count()).unwrap_or(0);
            let selection = cef::Range { from: end, to: end };
            host.ime_set_composition(
                Some(&text.as_str().into()),
                None,
                Some(&NO_RANGE),
                Some(&selection),
            );
        }
        Event::Ime(egui::ImeEvent::Commit(text)) => {
            host.ime_commit_text(Some(&text.as_str().into()), Some(&NO_RANGE), 0);
        }
        // egui reports the copy, cut and paste shortcuts as these events, so they run as the
        // matching commands of the focused frame.
        // CEF reads the system clipboard itself, so the pasted content is ignored.
        Event::Copy => with_focused_frame(browser, ImplFrame::copy),
        Event::Cut => with_focused_frame(browser, ImplFrame::cut),
        Event::Paste(_) | Event::PasteImage(_) => with_focused_frame(browser, ImplFrame::paste),
        _ => {}
    }
}

/// On macOS, runs Cmd-A, Cmd-Z and Shift-Cmd-Z as editing commands of the focused frame, and
/// returns whether `key` was one of them.
///
/// In a macOS app these shortcuts are actions of the app's Edit menu, so the page doesn't act on
/// them as key events.
fn mac_command(key: &KeyPress, browser: &Browser) -> bool {
    if !cfg!(target_os = "macos") || !key.modifiers.mac_cmd {
        return false;
    }
    let command: fn(&cef::Frame) = match (key.key, key.modifiers.shift) {
        (Key::A, false) => ImplFrame::select_all,
        (Key::Z, false) => ImplFrame::undo,
        (Key::Z, true) => ImplFrame::redo,
        _ => return false,
    };
    with_focused_frame(browser, command);
    true
}

/// Calls `f` with the frame of the page that has focus, which may be an `<iframe>`.
fn with_focused_frame(browser: &Browser, f: impl FnOnce(&cef::Frame)) {
    if let Some(frame) = browser.focused_frame() {
        f(&frame);
    }
}

/// The key of an [`Event::Key`], pressed or released.
struct KeyPress {
    key: Key,
    /// The key at the same physical position on a US keyboard layout.
    physical_key: Key,
    repeat: bool,
    modifiers: Modifiers,
}

impl KeyPress {
    /// Sends the key event, followed by a character event for each UTF-16 code unit of the
    /// `text` it typed.
    fn send(&self, host: &cef::BrowserHost, pressed: bool, text: &str) {
        let Some(windows_key_code) = keys::windows_key_code(self.key) else {
            // The key has no Windows virtual-key code, so only the text it typed is sent.
            for c in text.encode_utf16() {
                send_char(host, c, 0, self.modifiers);
            }
            return;
        };
        let mut modifiers = event_flags(self.modifiers);
        if self.repeat {
            modifiers |= bits(sys::cef_event_flags_t::EVENTFLAG_IS_REPEAT);
        }
        let event = KeyEvent {
            size: std::mem::size_of::<KeyEvent>(),
            type_: if pressed {
                sys::cef_key_event_type_t::KEYEVENT_RAWKEYDOWN
            } else {
                sys::cef_key_event_type_t::KEYEVENT_KEYUP
            }
            .into(),
            modifiers,
            windows_key_code,
            native_key_code: keys::native_key_code(self.physical_key),
            ..Default::default()
        };
        // egui sends no text for Enter, but pages expect a `\r` character event for it.
        let text: Vec<u16> = if text.is_empty() && self.key == Key::Enter {
            vec![u16::from(b'\r')]
        } else {
            text.encode_utf16().collect()
        };
        if cfg!(target_os = "macos") && text.is_empty() {
            self.send_mac(host, pressed, &event);
        } else {
            // Send a raw key down and then a character event per typed character, which is how
            // Chromium receives typed keys on Windows and Linux. This is done on macOS too,
            // because CEF there doesn't insert the `character` of a key event as text.
            host.send_key_event(Some(&event));
            if pressed {
                for &c in &text {
                    send_char(host, c, event.native_key_code, self.modifiers);
                }
            }
        }
    }

    /// Sends a key that types no text on macOS, such as an arrow key, Backspace or a shortcut.
    ///
    /// CEF turns the key event into an `NSEvent`, from which `AppKit` runs the key's editing
    /// command, such as `deleteBackward:`. That requires the key's `character` and a
    /// `KEYEVENT_KEYDOWN`, because a `KEYEVENT_RAWKEYDOWN` runs the command twice.
    fn send_mac(&self, host: &cef::BrowserHost, pressed: bool, event: &KeyEvent) {
        let shortcut = self.modifiers.ctrl || self.modifiers.alt || self.modifiers.mac_cmd;
        let character = keys::mac_function_char(self.key)
            // A shortcut types no text, but the page still needs its letter as the character.
            .or_else(|| plain_char(self.key).filter(|_| shortcut))
            .unwrap_or(0);
        host.send_key_event(Some(&KeyEvent {
            type_: if pressed {
                sys::cef_key_event_type_t::KEYEVENT_KEYDOWN.into()
            } else {
                event.type_
            },
            character,
            unmodified_character: plain_char(self.key).unwrap_or(character),
            ..*event
        }));
    }
}

/// The lowercase character that a letter or digit key types without modifiers.
fn plain_char(key: Key) -> Option<u16> {
    match key.name().as_bytes() {
        [c] if c.is_ascii_alphanumeric() => Some(u16::from(c.to_ascii_lowercase())),
        _ => None,
    }
}

/// Sends the UTF-16 code unit `c` as a `KEYEVENT_CHAR`. A character outside the Basic
/// Multilingual Plane takes two calls, one per surrogate.
fn send_char(host: &cef::BrowserHost, c: u16, native_key_code: i32, modifiers: Modifiers) {
    host.send_key_event(Some(&KeyEvent {
        size: std::mem::size_of::<KeyEvent>(),
        type_: sys::cef_key_event_type_t::KEYEVENT_CHAR.into(),
        modifiers: event_flags(modifiers),
        windows_key_code: i32::from(c),
        native_key_code,
        character: c,
        unmodified_character: c,
        ..Default::default()
    }));
}

/// CEF's event flags for `modifiers`.
fn event_flags(modifiers: Modifiers) -> u32 {
    use sys::cef_event_flags_t as F;
    [
        (modifiers.shift, F::EVENTFLAG_SHIFT_DOWN),
        (modifiers.ctrl, F::EVENTFLAG_CONTROL_DOWN),
        (modifiers.alt, F::EVENTFLAG_ALT_DOWN),
        (modifiers.mac_cmd, F::EVENTFLAG_COMMAND_DOWN),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |flags, (_, flag)| flags | bits(flag))
}

/// The bits of `flag` as a `u32`. The type CEF's bindings generate wraps an `i32` on Windows
/// and a `u32` elsewhere.
fn bits(flag: sys::cef_event_flags_t) -> u32 {
    u32::from_ne_bytes(flag.0.to_ne_bytes())
}

/// CEF's event flags for the mouse `buttons` that are held down.
fn button_flags(buttons: &[PointerButton]) -> u32 {
    use sys::cef_event_flags_t as F;
    buttons
        .iter()
        .map(|button| match button {
            PointerButton::Primary => bits(F::EVENTFLAG_LEFT_MOUSE_BUTTON),
            PointerButton::Secondary => bits(F::EVENTFLAG_RIGHT_MOUSE_BUTTON),
            PointerButton::Middle => bits(F::EVENTFLAG_MIDDLE_MOUSE_BUTTON),
            _ => 0,
        })
        .fold(0, |a, b| a | b)
}

/// CEF's type of `button`. `None` for the extra buttons, which CEF has no type for.
fn mouse_button(button: PointerButton) -> Option<cef::MouseButtonType> {
    use sys::cef_mouse_button_type_t as B;
    Some(
        match button {
            PointerButton::Primary => B::MBT_LEFT,
            PointerButton::Secondary => B::MBT_RIGHT,
            PointerButton::Middle => B::MBT_MIDDLE,
            _ => return None,
        }
        .into(),
    )
}
