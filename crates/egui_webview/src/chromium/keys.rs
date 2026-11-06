//! The key codes that a CEF key event carries for an egui key.

use egui::Key;
use winit::keyboard::KeyCode;

/// The Windows virtual-key code (`VK_*`) of `key`, or `None` if it has none.
///
/// CEF reads this code on every platform except macOS.
pub fn windows_key_code(key: Key) -> Option<i32> {
    let name = key.name();
    // `VK_A` to `VK_Z` and `VK_0` to `VK_9` equal the ASCII codes of the uppercase letters and
    // the digits.
    if let [c] = name.as_bytes() {
        return c.is_ascii_alphanumeric().then_some(i32::from(*c));
    }
    // `VK_F1` to `VK_F24` are 0x70 to 0x87.
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
        return (n <= 24).then_some(0x6F + n);
    }
    Some(match key {
        Key::Backspace => 0x08,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Escape => 0x1B,
        Key::Space => 0x20,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::End => 0x23,
        Key::Home => 0x24,
        Key::ArrowLeft => 0x25,
        Key::ArrowUp => 0x26,
        Key::ArrowRight => 0x27,
        Key::ArrowDown => 0x28,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::SuperLeft => 0x5B,
        Key::SuperRight => 0x5C,
        Key::ShiftLeft => 0xA0,
        Key::ShiftRight => 0xA1,
        Key::ControlLeft => 0xA2,
        Key::ControlRight => 0xA3,
        Key::AltLeft => 0xA4,
        Key::AltRight => 0xA5,
        Key::Clear => 0x0C,
        Key::Pause => 0x13,
        Key::CapsLock => 0x14,
        Key::PrintScreen => 0x2C,
        Key::Help => 0x2F,
        Key::Menu => 0x5D,
        Key::Sleep => 0x5F,
        Key::NumLock => 0x90,
        Key::ScrollLock => 0x91,
        Key::BrowserBack => 0xA6,
        Key::BrowserForward => 0xA7,
        Key::BrowserRefresh => 0xA8,
        Key::BrowserStop => 0xA9,
        Key::BrowserSearch => 0xAA,
        Key::BrowserFavorites => 0xAB,
        Key::BrowserHome => 0xAC,
        Key::AudioVolumeMute => 0xAD,
        Key::AudioVolumeDown => 0xAE,
        Key::AudioVolumeUp => 0xAF,
        Key::MediaTrackNext => 0xB0,
        Key::MediaTrackPrevious => 0xB1,
        Key::MediaStop => 0xB2,
        Key::MediaPlayPause => 0xB3,
        Key::LaunchMail => 0xB4,
        Key::LaunchApp1 => 0xB6,
        Key::LaunchApp2 => 0xB7,
        Key::Semicolon | Key::Colon => 0xBA,
        Key::Equals | Key::Plus => 0xBB,
        Key::Comma => 0xBC,
        Key::Minus => 0xBD,
        Key::Period => 0xBE,
        Key::Slash | Key::Questionmark => 0xBF,
        Key::Backtick => 0xC0,
        Key::OpenBracket | Key::OpenCurlyBracket => 0xDB,
        Key::Backslash | Key::Pipe => 0xDC,
        Key::CloseBracket | Key::CloseCurlyBracket => 0xDD,
        Key::Quote => 0xDE,
        Key::IntlBackslash => 0xE2,
        Key::Exclamationmark => 0x31,
        _ => return None,
    })
}

/// The `native_key_code` of a CEF key event for the physical key at `key`'s position on a US
/// keyboard layout, or 0 if it is unknown.
///
/// egui doesn't keep the platform's key code, so this gets the scancode of that physical key from
/// winit: a macOS virtual key code, a Windows scancode, or a Linux evdev code.
pub fn native_key_code(key: Key) -> i32 {
    let Some(scancode) = scancode(winit_key_code(key)).and_then(|code| i32::try_from(code).ok())
    else {
        return 0;
    };
    if cfg!(target_os = "windows") {
        // The `lParam` of `WM_KEYDOWN`: a repeat count of 1, the scancode in bits 16 to 23,
        // and bit 24 for an extended key. winit gives extended keys a 0xE0 prefix byte.
        let extended = i32::from(scancode >> 8 == 0xE0);
        1 | ((scancode & 0xFF) << 16) | (extended << 24)
    } else if cfg!(target_os = "macos") {
        scancode
    } else {
        // An X11 key code is the evdev code plus 8.
        scancode + 8
    }
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn scancode(code: KeyCode) -> Option<u32> {
    use winit::platform::scancode::PhysicalKeyExtScancode as _;
    code.to_scancode()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn scancode(_code: KeyCode) -> Option<u32> {
    None
}

/// The winit code of the physical key that egui-winit maps to `key`, which reverses
/// egui-winit's mapping.
///
/// A symbol that needs Shift, such as `Key::Colon`, maps to the key that types it on a US
/// keyboard layout.
fn winit_key_code(key: Key) -> KeyCode {
    // Each arm maps one or more egui keys to a winit key: `EguiKey | OtherEguiKey => WinitKey`.
    macro_rules! codes {
        ($($($egui:ident)|+ => $winit:ident),* $(,)?) => {
            match key {
                $($(Key::$egui)|+ => KeyCode::$winit,)*
            }
        };
    }
    codes! {
        A => KeyA, B => KeyB, C => KeyC, D => KeyD, E => KeyE, F => KeyF, G => KeyG,
        H => KeyH, I => KeyI, J => KeyJ, K => KeyK, L => KeyL, M => KeyM, N => KeyN,
        O => KeyO, P => KeyP, Q => KeyQ, R => KeyR, S => KeyS, T => KeyT, U => KeyU,
        V => KeyV, W => KeyW, X => KeyX, Y => KeyY, Z => KeyZ,
        Num0 => Digit0, Num1 | Exclamationmark => Digit1, Num2 => Digit2, Num3 => Digit3,
        Num4 => Digit4, Num5 => Digit5, Num6 => Digit6, Num7 => Digit7, Num8 => Digit8,
        Num9 => Digit9,
        F1 => F1, F2 => F2, F3 => F3, F4 => F4, F5 => F5, F6 => F6, F7 => F7, F8 => F8,
        F9 => F9, F10 => F10, F11 => F11, F12 => F12, F13 => F13, F14 => F14, F15 => F15,
        F16 => F16, F17 => F17, F18 => F18, F19 => F19, F20 => F20, F21 => F21, F22 => F22,
        F23 => F23, F24 => F24, F25 => F25, F26 => F26, F27 => F27, F28 => F28, F29 => F29,
        F30 => F30, F31 => F31, F32 => F32, F33 => F33, F34 => F34, F35 => F35,
        ArrowDown => ArrowDown, ArrowLeft => ArrowLeft, ArrowRight => ArrowRight,
        ArrowUp => ArrowUp, Escape => Escape, Tab => Tab, Backspace => Backspace,
        Enter => Enter, Space => Space, Insert => Insert, Delete => Delete, Home => Home,
        End => End, PageUp => PageUp, PageDown => PageDown, Copy => Copy, Cut => Cut,
        Paste => Paste,
        Comma => Comma, Period => Period, Minus => Minus, Quote => Quote,
        Semicolon | Colon => Semicolon, Equals | Plus => Equal, Slash | Questionmark => Slash,
        Backslash | Pipe => Backslash, Backtick => Backquote,
        OpenBracket | OpenCurlyBracket => BracketLeft,
        CloseBracket | CloseCurlyBracket => BracketRight,
        IntlBackslash => IntlBackslash,
        ShiftLeft => ShiftLeft, ShiftRight => ShiftRight, ControlLeft => ControlLeft,
        ControlRight => ControlRight, AltLeft => AltLeft, AltRight => AltRight,
        SuperLeft => SuperLeft, SuperRight => SuperRight,
        CapsLock => CapsLock, NumLock => NumLock, ScrollLock => ScrollLock,
        PrintScreen => PrintScreen, Pause => Pause, Menu => ContextMenu,
        BrowserBack => BrowserBack, BrowserForward => BrowserForward,
        BrowserRefresh => BrowserRefresh, BrowserSearch => BrowserSearch,
        BrowserHome => BrowserHome, BrowserFavorites => BrowserFavorites,
        BrowserStop => BrowserStop, MediaPlayPause => MediaPlayPause,
        MediaTrackNext => MediaTrackNext, MediaTrackPrevious => MediaTrackPrevious,
        MediaStop => MediaStop, AudioVolumeMute => AudioVolumeMute,
        AudioVolumeDown => AudioVolumeDown, AudioVolumeUp => AudioVolumeUp,
        LaunchMail => LaunchMail, LaunchApp1 => LaunchApp1, LaunchApp2 => LaunchApp2,
        Fn => Fn, Eject => Eject, Help => Help, Power => Power, Sleep => Sleep,
        Clear => NumpadClear,
    }
}

/// The character that `NSEvent.characters` holds on macOS for a function or control key, such
/// as `NSUpArrowFunctionKey` for the up arrow.
pub fn mac_function_char(key: Key) -> Option<u16> {
    if let Some(n) = key
        .name()
        .strip_prefix('F')
        .and_then(|n| n.parse::<u16>().ok())
    {
        // `NSF1FunctionKey` is 0xF704, and the codes of the F keys after it are consecutive.
        return Some(0xF703 + n);
    }
    Some(match key {
        Key::Backspace => 0x7F,
        Key::Tab => 0x09,
        Key::Enter => 0x0D,
        Key::Escape => 0x1B,
        Key::ArrowUp => 0xF700,
        Key::ArrowDown => 0xF701,
        Key::ArrowLeft => 0xF702,
        Key::ArrowRight => 0xF703,
        Key::Insert => 0xF727,
        Key::Delete => 0xF728,
        Key::Home => 0xF729,
        Key::End => 0xF72B,
        Key::PageUp => 0xF72C,
        Key::PageDown => 0xF72D,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_key_codes() {
        assert_eq!(windows_key_code(Key::A), Some(0x41));
        assert_eq!(windows_key_code(Key::Num7), Some(0x37));
        assert_eq!(windows_key_code(Key::F1), Some(0x70));
        assert_eq!(windows_key_code(Key::F24), Some(0x87));
        assert_eq!(windows_key_code(Key::F25), None);
        assert_eq!(windows_key_code(Key::Questionmark), Some(0xBF));
    }

    #[test]
    fn native_key_codes() {
        // The native key codes of A, Backspace and the left arrow on each platform.
        let expected = if cfg!(target_os = "macos") {
            [0x00, 0x33, 0x7B]
        } else if cfg!(target_os = "windows") {
            [
                (0x1E << 16) | 1,
                (0x0E << 16) | 1,
                (0x4B << 16) | (1 << 24) | 1,
            ]
        } else {
            [30 + 8, 14 + 8, 105 + 8]
        };
        let keys = [Key::A, Key::Backspace, Key::ArrowLeft];
        assert_eq!(keys.map(native_key_code), expected);
    }

    #[test]
    fn mac_function_chars() {
        assert_eq!(mac_function_char(Key::F1), Some(0xF704));
        assert_eq!(mac_function_char(Key::A), None);
    }
}
