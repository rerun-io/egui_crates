//! Forwarding egui's pointer, wheel, keyboard, text, IME and clipboard events to a CEF browser.
//!
//! The page's widget records a [`Batch`] of input in each pass, and [`Input::replay`] sends it
//! to the page after the pass.

use std::iter::Peekable;

use cef::{
    Browser, BrowserHost, CefString, Frame, ImplBrowser as _, ImplBrowserHost, ImplFrame as _,
    KeyEvent, MouseButtonType, MouseEvent, sys,
};
use egui::{
    Event, ImeEvent, Key, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Response, Ui, Vec2,
};

use super::keys::KeyExt as _;

/// What the page's widget saw in one pass.
///
/// The widget records it during the pass, and [`Input::replay`] sends it to the page after the
/// pass. Everything is copied here, because by then egui's input has moved on.
#[derive(Debug)]
pub struct Batch {
    events: Vec<Event>,

    /// The page's rect.
    rect: Rect,

    /// False while a layer above the page, such as a window or a popup, covers the pointer.
    hovered: bool,

    /// Whether the widget had egui's focus before these events.
    focused: bool,

    /// Whether the keyboard, text, IME and clipboard events go to the page.
    forward_keys: bool,

    time: f64,
    hover_pos: Option<Pos2>,
    modifiers: Modifiers,

    /// The points scrolled per line of an [`egui::MouseWheelUnit::Line`] wheel event.
    points_per_line: f32,
}

impl Batch {
    /// Records this pass's input for the page in `response.rect`.
    ///
    /// It also does the two things that the input means for egui itself. A press on the page
    /// gives the widget egui's focus, and a wheel event over the page isn't egui's to scroll.
    pub fn record(ui: &Ui, response: &Response, forward_keys: bool) -> Self {
        let hovered = response.contains_pointer();
        let focused = response.has_focus();
        let points_per_line = ui
            .ctx()
            .options(|options| options.input_options.line_scroll_speed);

        // The page scrolls itself, so an enclosing `ScrollArea` mustn't scroll as well.
        if hovered {
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }

        let batch = ui.input(|i| Self {
            events: i.events.clone(),
            rect: response.rect,
            hovered,
            focused,
            forward_keys,
            time: i.time,
            hover_pos: i.pointer.hover_pos(),
            modifiers: i.modifiers,
            points_per_line,
        });

        let pressed = batch.events.iter().any(|event| {
            matches!(*event, Event::PointerButton { button, pressed: true, .. }
                if MouseButton::from_egui(button).is_some())
        });
        if hovered && pressed {
            response.request_focus();
        }

        batch
    }

    /// Whether the pass had no input. Such a batch has nothing to replay.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Where the pointer is, for the events that don't say.
    fn pointer_pos(&self) -> Pos2 {
        self.hover_pos.unwrap_or(self.rect.min)
    }

    /// The CEF mouse event for a pointer at `pos`, in egui's coordinates, with the keyboard
    /// `modifiers` and the event flags of the mouse buttons that are held down.
    fn mouse_event(&self, pos: Pos2, modifiers: Modifiers, buttons_down: u32) -> MouseEvent {
        // CEF takes whole points, relative to the page's corner.
        let pos = (pos - self.rect.min).round();
        #[expect(clippy::cast_possible_truncation)]
        let (x, y) = (pos.x as i32, pos.y as i32);
        MouseEvent {
            x,
            y,
            modifiers: event_flags(modifiers) | buttons_down,
        }
    }
}

/// The pointer state that forwarding input to one page carries from pass to pass.
#[derive(Debug, Default)]
pub struct Input {
    /// The event flags of the mouse buttons pressed on the page and not released yet.
    ///
    /// While one is down, the page keeps getting pointer events outside its rect, like with a
    /// browser's implicit pointer capture.
    buttons_down: u32,

    /// Whether the page was sent a mouse move since the last mouse leave.
    pointer_inside: bool,

    /// The last press, for counting double and triple clicks.
    last_click: Option<Click>,

    /// The fraction of the wheel delta not sent yet, since CEF takes whole points.
    wheel_remainder: Vec2,
}

/// A press of a mouse button on the page.
#[derive(Debug, Clone, Copy)]
struct Click {
    time: f64,
    pos: Pos2,
    button: MouseButton,

    /// 1 for a single click, 2 for the second click of a double click, and so on.
    count: i32,
}

impl Input {
    /// Sends the input of one pass to the page. That is the pointer events, and if the widget
    /// has egui's focus, the keyboard, text, IME and clipboard events too.
    pub fn replay(&mut self, browser: &Browser, batch: &Batch) {
        let Some(host) = browser.host() else {
            return;
        };
        let mut focused = batch.focused;

        let mut events = batch.events.iter().peekable();
        while let Some(event) = events.next() {
            match *event {
                Event::PointerMoved(pos) => self.pointer_moved(&host, batch, pos),
                Event::PointerGone => self.pointer_left(&host, batch, batch.pointer_pos()),
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    let Some(button) = MouseButton::from_egui(button) else {
                        continue;
                    };
                    if !pressed {
                        self.button_released(&host, batch, button, pos, modifiers);
                    } else if batch.hovered {
                        self.button_pressed(&host, batch, button, pos, modifiers);
                        // The press gives the widget egui's focus, so the keys after it in the
                        // same pass already go to the page.
                        focused = true;
                    }
                }
                Event::MouseWheel {
                    unit,
                    delta,
                    modifiers,
                    ..
                } => self.wheel(&host, batch, unit, delta, modifiers),
                _ if batch.forward_keys && focused => {
                    forward_keyboard_event(browser, &host, event, &mut events);
                }
                _ => {}
            }
        }
    }

    fn pointer_moved(&mut self, host: &BrowserHost, batch: &Batch, pos: Pos2) {
        // A held button keeps the pointer captured by the page.
        if batch.hovered || self.buttons_down != NO_BUTTONS {
            host.mouse_move(&batch.mouse_event(pos, batch.modifiers, self.buttons_down));
            self.pointer_inside = true;
        } else {
            self.pointer_left(host, batch, pos);
        }
    }

    /// Tells the page that the pointer left it at `pos`, unless the page was already told.
    fn pointer_left(&mut self, host: &BrowserHost, batch: &Batch, pos: Pos2) {
        if std::mem::take(&mut self.pointer_inside) {
            host.mouse_leave(&batch.mouse_event(pos, batch.modifiers, NO_BUTTONS));
        }
    }

    fn button_pressed(
        &mut self,
        host: &BrowserHost,
        batch: &Batch,
        button: MouseButton,
        pos: Pos2,
        modifiers: Modifiers,
    ) {
        let click_count = self.count_click(batch.time, pos, button);
        self.buttons_down |= button.flag;
        let event = batch.mouse_event(pos, modifiers, self.buttons_down);
        host.mouse_down(&event, button, click_count);
    }

    fn button_released(
        &mut self,
        host: &BrowserHost,
        batch: &Batch,
        button: MouseButton,
        pos: Pos2,
        modifiers: Modifiers,
    ) {
        // The page only gets the release of a press that it got.
        let was_pressed = self.buttons_down & button.flag != 0;
        if !was_pressed {
            return;
        }
        self.buttons_down &= !button.flag;
        let click_count = self.last_click.map_or(1, |click| click.count);
        let event = batch.mouse_event(pos, modifiers, self.buttons_down);
        host.mouse_up(&event, button, click_count);
    }

    /// Scrolls the page under the pointer.
    fn wheel(
        &mut self,
        host: &BrowserHost,
        batch: &Batch,
        unit: MouseWheelUnit,
        delta: Vec2,
        modifiers: Modifiers,
    ) {
        if !batch.hovered {
            return;
        }
        let points = match unit {
            MouseWheelUnit::Point => delta,
            MouseWheelUnit::Line => delta * batch.points_per_line,
            MouseWheelUnit::Page => delta * batch.rect.height(),
        };
        // CEF takes whole points, so the fraction waits for the next wheel event.
        let points = points + self.wheel_remainder;
        let whole_points = points.round();
        self.wheel_remainder = points - whole_points;

        let event = batch.mouse_event(batch.pointer_pos(), modifiers, NO_BUTTONS);
        host.mouse_wheel(&event, whole_points);
    }

    /// The click count of a press, which is 2 for the second click of a double click.
    ///
    /// A windowless browser doesn't count clicks, so this counts them like the OS would.
    fn count_click(&mut self, time: f64, pos: Pos2, button: MouseButton) -> i32 {
        const MAX_DELAY: f64 = 0.5;
        const MAX_DISTANCE: f32 = 4.0;

        let continues = |last: &Click| {
            last.button == button
                && time - last.time < MAX_DELAY
                && last.pos.distance(pos) < MAX_DISTANCE
        };
        let count = self
            .last_click
            .filter(continues)
            .map_or(1, |last| last.count + 1);
        self.last_click = Some(Click {
            time,
            pos,
            button,
            count,
        });
        count
    }
}

/// Forwards `event` to the focused page, if it is a keyboard, text, IME or clipboard event.
///
/// `rest` holds the events after `event`. The text that a key press typed is taken from it.
fn forward_keyboard_event<'a>(
    browser: &Browser,
    host: &BrowserHost,
    event: &Event,
    rest: &mut Peekable<impl Iterator<Item = &'a Event>>,
) {
    match event {
        &Event::Key {
            key,
            physical_key,
            pressed,
            repeat,
            modifiers,
        } => {
            // egui emits the `Event::Text` that a key press typed right after its `Event::Key`,
            // so the two are sent together.
            let text = match rest.peek() {
                Some(Event::Text(text)) if pressed => {
                    rest.next();
                    text.as_str()
                }
                _ => "",
            };
            if pressed && let Some(command) = mac_edit_command(key, modifiers) {
                run_in_focused_frame(browser, command);
            } else {
                let keystroke = Keystroke {
                    key,
                    physical_key: physical_key.unwrap_or(key),
                    pressed,
                    repeat,
                    modifiers,
                };
                keystroke.send(host, text);
            }
        }
        Event::Text(text) => host.type_text(text, NO_NATIVE_KEY_CODE, Modifiers::NONE),
        Event::Ime(ImeEvent::Preedit { text, .. }) => host.ime_preedit(text),
        Event::Ime(ImeEvent::Commit(text)) => host.ime_commit(text),

        // egui reports the copy, cut and paste shortcuts as these events, so they run as the
        // matching commands of the focused frame.
        Event::Copy => run_in_focused_frame(browser, Frame::copy),
        Event::Cut => run_in_focused_frame(browser, Frame::cut),
        // CEF reads the system clipboard itself, so the pasted content is ignored.
        Event::Paste(_) | Event::PasteImage(_) => run_in_focused_frame(browser, Frame::paste),

        _ => {}
    }
}

/// The editing command that a key press runs on macOS instead of being sent as a key event:
/// Cmd-A, Cmd-Z and Shift-Cmd-Z.
///
/// In a macOS app these shortcuts are actions of the app's Edit menu, so the page doesn't act on
/// them as key events.
fn mac_edit_command(key: Key, modifiers: Modifiers) -> Option<fn(&Frame)> {
    if !cfg!(target_os = "macos") || !modifiers.mac_cmd {
        return None;
    }
    match (key, modifiers.shift) {
        (Key::A, false) => Some(Frame::select_all),
        (Key::Z, false) => Some(Frame::undo),
        (Key::Z, true) => Some(Frame::redo),
        _ => None,
    }
}

/// Runs `command` in the frame of the page that has focus, which may be an `<iframe>`.
fn run_in_focused_frame(browser: &Browser, command: fn(&Frame)) {
    if let Some(frame) = browser.focused_frame() {
        command(&frame);
    }
}

/// One [`Event::Key`]: a key going down or up.
struct Keystroke {
    key: Key,

    /// The key at the same physical position on a US keyboard layout.
    physical_key: Key,

    pressed: bool,
    repeat: bool,
    modifiers: Modifiers,
}

impl Keystroke {
    /// Sends the key event, and after a press, the `text` that it typed.
    fn send(&self, host: &BrowserHost, text: &str) {
        use sys::{cef_event_flags_t as Flag, cef_key_event_type_t as Type};

        let Some(windows_key_code) = self.key.windows_key_code() else {
            // The key has no Windows virtual-key code, so only the text it typed is sent.
            host.type_text(text, NO_NATIVE_KEY_CODE, self.modifiers);
            return;
        };
        // egui sends no text for Enter, but pages expect a `\r` character event for it.
        let text = if text.is_empty() && self.key == Key::Enter {
            "\r"
        } else {
            text
        };

        // A raw key down and then a character event per typed character is how Chromium
        // receives typed keys on Windows and Linux. This is done on macOS too, because CEF
        // there doesn't insert the `character` of a key event as text.
        let mut event = KeyEvent {
            size: size_of::<KeyEvent>(),
            type_: if self.pressed {
                Type::KEYEVENT_RAWKEYDOWN
            } else {
                Type::KEYEVENT_KEYUP
            }
            .into(),
            modifiers: event_flags(self.modifiers),
            windows_key_code,
            native_key_code: self.physical_key.native_key_code(),
            ..Default::default()
        };
        if self.repeat {
            event.modifiers |= bits(Flag::EVENTFLAG_IS_REPEAT);
        }
        if cfg!(target_os = "macos") && text.is_empty() {
            self.adapt_to_appkit(&mut event);
        }

        host.send_key_event(Some(&event));
        if self.pressed {
            host.type_text(text, event.native_key_code, self.modifiers);
        }
    }

    /// Turns `event` into the one that macOS needs for a key that types no text, such as an
    /// arrow key, Backspace or a shortcut.
    ///
    /// CEF turns the key event into an `NSEvent`, from which `AppKit` runs the key's editing
    /// command, such as `deleteBackward:`. That requires the key's `character` and a
    /// `KEYEVENT_KEYDOWN`, because a `KEYEVENT_RAWKEYDOWN` runs the command twice.
    fn adapt_to_appkit(&self, event: &mut KeyEvent) {
        let unmodified = self.key.unmodified_char();
        let is_shortcut = self.modifiers.ctrl || self.modifiers.alt || self.modifiers.mac_cmd;
        // A shortcut types no text, but the page still needs its letter as the character.
        let shortcut_char = unmodified.filter(|_| is_shortcut);
        let character = self.key.mac_function_char().or(shortcut_char).unwrap_or(0);

        if self.pressed {
            event.type_ = sys::cef_key_event_type_t::KEYEVENT_KEYDOWN.into();
        }
        event.character = character;
        event.unmodified_character = unmodified.unwrap_or(character);
    }
}

/// A mouse button that CEF has a type for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MouseButton {
    type_: MouseButtonType,

    /// The event flag that says the button is held down.
    flag: u32,
}

impl MouseButton {
    /// `None` for the extra buttons, which CEF has no type for.
    fn from_egui(button: PointerButton) -> Option<Self> {
        use sys::{cef_event_flags_t as Flag, cef_mouse_button_type_t as Type};

        let (type_, flag) = match button {
            PointerButton::Primary => (Type::MBT_LEFT, Flag::EVENTFLAG_LEFT_MOUSE_BUTTON),
            PointerButton::Secondary => (Type::MBT_RIGHT, Flag::EVENTFLAG_RIGHT_MOUSE_BUTTON),
            PointerButton::Middle => (Type::MBT_MIDDLE, Flag::EVENTFLAG_MIDDLE_MOUSE_BUTTON),
            PointerButton::Extra1 | PointerButton::Extra2 => return None,
        };
        Some(Self {
            type_: type_.into(),
            flag: bits(flag),
        })
    }
}

/// The event flags of a mouse event during which no mouse button is held down.
const NO_BUTTONS: u32 = 0;

/// The `native_key_code` of a character event that no key event came with.
const NO_NATIVE_KEY_CODE: i32 = 0;

/// The invalid range, which CEF's IME methods read as "no replacement range".
///
/// CEF ignores an IME call whose range argument is `None`, so this is passed instead.
const NO_RANGE: cef::Range = cef::Range {
    from: u32::MAX,
    to: u32::MAX,
};

/// [`BrowserHost`]'s methods for sending input, with Rust types in place of the C conventions of
/// their arguments.
trait BrowserHostExt: ImplBrowserHost {
    fn mouse_move(&self, event: &MouseEvent) {
        let mouse_leave = false;
        self.send_mouse_move_event(Some(event), mouse_leave.into());
    }

    fn mouse_leave(&self, event: &MouseEvent) {
        let mouse_leave = true;
        self.send_mouse_move_event(Some(event), mouse_leave.into());
    }

    fn mouse_down(&self, event: &MouseEvent, button: MouseButton, click_count: i32) {
        let mouse_up = false;
        self.send_mouse_click_event(Some(event), button.type_, mouse_up.into(), click_count);
    }

    fn mouse_up(&self, event: &MouseEvent, button: MouseButton, click_count: i32) {
        let mouse_up = true;
        self.send_mouse_click_event(Some(event), button.type_, mouse_up.into(), click_count);
    }

    /// Scrolls by `delta`, which is in whole points.
    fn mouse_wheel(&self, event: &MouseEvent, delta: Vec2) {
        #[expect(clippy::cast_possible_truncation)]
        let (x, y) = (delta.x as i32, delta.y as i32);
        self.send_mouse_wheel_event(Some(event), x, y);
    }

    /// Sends `text` as one `KEYEVENT_CHAR` per UTF-16 code unit. A character outside the Basic
    /// Multilingual Plane takes two, one per surrogate.
    fn type_text(&self, text: &str, native_key_code: i32, modifiers: Modifiers) {
        for c in text.encode_utf16() {
            self.send_key_event(Some(&KeyEvent {
                size: size_of::<KeyEvent>(),
                type_: sys::cef_key_event_type_t::KEYEVENT_CHAR.into(),
                modifiers: event_flags(modifiers),
                windows_key_code: i32::from(c),
                native_key_code,
                character: c,
                unmodified_character: c,
                ..Default::default()
            }));
        }
    }

    /// Shows `text` as the text that an input method is composing, with the caret at its end.
    fn ime_preedit(&self, text: &str) {
        let end = u32::try_from(text.encode_utf16().count()).unwrap_or(0);
        let caret = cef::Range { from: end, to: end };
        self.ime_set_composition(
            Some(&CefString::from(text)),
            None,
            Some(&NO_RANGE),
            Some(&caret),
        );
    }

    /// Inserts `text` as the result of an input method's composition.
    fn ime_commit(&self, text: &str) {
        let relative_cursor_pos = 0;
        self.ime_commit_text(
            Some(&CefString::from(text)),
            Some(&NO_RANGE),
            relative_cursor_pos,
        );
    }
}

impl BrowserHostExt for BrowserHost {}

/// CEF's event flags for the keyboard `modifiers`.
fn event_flags(modifiers: Modifiers) -> u32 {
    use sys::cef_event_flags_t as Flag;

    let flag_if = |down: bool, flag: Flag| if down { bits(flag) } else { 0 };
    flag_if(modifiers.shift, Flag::EVENTFLAG_SHIFT_DOWN)
        | flag_if(modifiers.ctrl, Flag::EVENTFLAG_CONTROL_DOWN)
        | flag_if(modifiers.alt, Flag::EVENTFLAG_ALT_DOWN)
        | flag_if(modifiers.mac_cmd, Flag::EVENTFLAG_COMMAND_DOWN)
}

/// The bits of `flag` as the `u32` that CEF's events take. The type that CEF's bindings generate
/// for the flags wraps an `i32` on Windows and a `u32` elsewhere.
fn bits(flag: sys::cef_event_flags_t) -> u32 {
    u32::from_ne_bytes(flag.0.to_ne_bytes())
}
