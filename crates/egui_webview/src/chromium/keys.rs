//! The key codes that a CEF key event carries for an egui key.

use egui::Key;
use winit::keyboard::KeyCode;

/// The codes that identify an egui [`Key`] in a CEF key event.
pub trait KeyExt {
    /// The Windows virtual-key code (`VK_*`) of the key, or `None` if it has none.
    ///
    /// CEF reads this code on every platform except macOS.
    fn windows_key_code(self) -> Option<i32>;

    /// The `native_key_code` of a CEF key event for the physical key at this key's position on a
    /// US keyboard layout, or 0 if it is unknown.
    ///
    /// egui doesn't keep the platform's key code, so this gets the scancode of that physical key
    /// from winit: a macOS virtual key code, a Windows scancode, or a Linux evdev code.
    fn native_key_code(self) -> i32;

    /// The character that `NSEvent.characters` holds on macOS for a function or control key,
    /// such as `NSUpArrowFunctionKey` for the up arrow.
    fn mac_function_char(self) -> Option<u16>;

    /// The lowercase character that a letter or digit key types without modifiers.
    fn unmodified_char(self) -> Option<u16>;
}

impl KeyExt for Key {
    fn windows_key_code(self) -> Option<i32> {
        // `VK_0` to `VK_9` and `VK_A` to `VK_Z` equal the ASCII codes of the digits and of the
        // uppercase letters.
        if let Some(c) = ascii_alphanumeric(self) {
            return Some(i32::from(c));
        }
        // `VK_F1` to `VK_F24` are 0x70 to 0x87. The F keys after them have no code.
        if let Some(n) = function_key_number(self) {
            return (n <= 24).then_some(0x6F + i32::from(n));
        }
        Some(match self {
            Self::Backspace => 0x08,
            Self::Tab => 0x09,
            Self::Clear => 0x0C,
            Self::Enter => 0x0D,
            Self::Pause => 0x13,
            Self::CapsLock => 0x14,
            Self::Escape => 0x1B,
            Self::Space => 0x20,
            Self::PageUp => 0x21,
            Self::PageDown => 0x22,
            Self::End => 0x23,
            Self::Home => 0x24,
            Self::ArrowLeft => 0x25,
            Self::ArrowUp => 0x26,
            Self::ArrowRight => 0x27,
            Self::ArrowDown => 0x28,
            Self::PrintScreen => 0x2C,
            Self::Insert => 0x2D,
            Self::Delete => 0x2E,
            Self::Help => 0x2F,
            Self::SuperLeft => 0x5B,
            Self::SuperRight => 0x5C,
            Self::Menu => 0x5D,
            Self::Sleep => 0x5F,
            Self::NumLock => 0x90,
            Self::ScrollLock => 0x91,
            Self::ShiftLeft => 0xA0,
            Self::ShiftRight => 0xA1,
            Self::ControlLeft => 0xA2,
            Self::ControlRight => 0xA3,
            Self::AltLeft => 0xA4,
            Self::AltRight => 0xA5,
            Self::BrowserBack => 0xA6,
            Self::BrowserForward => 0xA7,
            Self::BrowserRefresh => 0xA8,
            Self::BrowserStop => 0xA9,
            Self::BrowserSearch => 0xAA,
            Self::BrowserFavorites => 0xAB,
            Self::BrowserHome => 0xAC,
            Self::AudioVolumeMute => 0xAD,
            Self::AudioVolumeDown => 0xAE,
            Self::AudioVolumeUp => 0xAF,
            Self::MediaTrackNext => 0xB0,
            Self::MediaTrackPrevious => 0xB1,
            Self::MediaStop => 0xB2,
            Self::MediaPlayPause => 0xB3,
            Self::LaunchMail => 0xB4,
            Self::LaunchApp1 => 0xB6,
            Self::LaunchApp2 => 0xB7,

            // Punctuation has the code of the key that types it on a US keyboard layout, so a
            // symbol that needs Shift shares the code of the symbol below it.
            Self::Exclamationmark => 0x31,
            Self::Semicolon | Self::Colon => 0xBA,
            Self::Equals | Self::Plus => 0xBB,
            Self::Comma => 0xBC,
            Self::Minus => 0xBD,
            Self::Period => 0xBE,
            Self::Slash | Self::Questionmark => 0xBF,
            Self::Backtick => 0xC0,
            Self::OpenBracket | Self::OpenCurlyBracket => 0xDB,
            Self::Backslash | Self::Pipe => 0xDC,
            Self::CloseBracket | Self::CloseCurlyBracket => 0xDD,
            Self::Quote => 0xDE,
            Self::IntlBackslash => 0xE2,

            _ => return None,
        })
    }

    fn native_key_code(self) -> i32 {
        let Some(scancode) = scancode(physical_key_code(self)) else {
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

    fn mac_function_char(self) -> Option<u16> {
        // `NSF1FunctionKey` is 0xF704, and the codes of the F keys after it are consecutive.
        if let Some(n) = function_key_number(self) {
            return Some(0xF703 + u16::from(n));
        }
        Some(match self {
            Self::Tab => 0x09,
            Self::Enter => 0x0D,
            Self::Escape => 0x1B,
            Self::Backspace => 0x7F,
            Self::ArrowUp => 0xF700,
            Self::ArrowDown => 0xF701,
            Self::ArrowLeft => 0xF702,
            Self::ArrowRight => 0xF703,
            Self::Insert => 0xF727,
            Self::Delete => 0xF728,
            Self::Home => 0xF729,
            Self::End => 0xF72B,
            Self::PageUp => 0xF72C,
            Self::PageDown => 0xF72D,
            _ => return None,
        })
    }

    fn unmodified_char(self) -> Option<u16> {
        ascii_alphanumeric(self).map(|c| u16::from(c.to_ascii_lowercase()))
    }
}

/// The ASCII code of a digit key's digit, or of a letter key's uppercase letter.
fn ascii_alphanumeric(key: Key) -> Option<u8> {
    // egui names these keys, and no others, by their character: "0" to "9" and "A" to "Z".
    match key.name().as_bytes() {
        &[c] if c.is_ascii_alphanumeric() => Some(c),
        _ => None,
    }
}

/// The number of a function key: 1 for F1, up to 35 for F35.
fn function_key_number(key: Key) -> Option<u8> {
    // egui names these keys, and no others, "F" and a number: "F1" to "F35".
    key.name().strip_prefix('F')?.parse().ok()
}

/// winit's scancode for the physical key `code`, or `None` if the platform has none for it.
#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
fn scancode(code: KeyCode) -> Option<i32> {
    use winit::platform::scancode::PhysicalKeyExtScancode as _;
    i32::try_from(code.to_scancode()?).ok()
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn scancode(_code: KeyCode) -> Option<i32> {
    None
}

/// The winit code of the physical key that egui-winit maps to `key`, which reverses
/// egui-winit's mapping.
///
/// A symbol that needs Shift, such as `Key::Colon`, maps to the key that types it on a US
/// keyboard layout.
fn physical_key_code(key: Key) -> KeyCode {
    match key {
        Key::ArrowDown => KeyCode::ArrowDown,
        Key::ArrowLeft => KeyCode::ArrowLeft,
        Key::ArrowRight => KeyCode::ArrowRight,
        Key::ArrowUp => KeyCode::ArrowUp,

        Key::Escape => KeyCode::Escape,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Enter => KeyCode::Enter,
        Key::Space => KeyCode::Space,

        Key::Insert => KeyCode::Insert,
        Key::Delete => KeyCode::Delete,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,

        Key::Copy => KeyCode::Copy,
        Key::Cut => KeyCode::Cut,
        Key::Paste => KeyCode::Paste,

        // Punctuation
        Key::Comma => KeyCode::Comma,
        Key::Period => KeyCode::Period,
        Key::Minus => KeyCode::Minus,
        Key::Quote => KeyCode::Quote,
        Key::Semicolon | Key::Colon => KeyCode::Semicolon,
        Key::Equals | Key::Plus => KeyCode::Equal,
        Key::Slash | Key::Questionmark => KeyCode::Slash,
        Key::Backslash | Key::Pipe => KeyCode::Backslash,
        Key::Backtick => KeyCode::Backquote,
        Key::OpenBracket | Key::OpenCurlyBracket => KeyCode::BracketLeft,
        Key::CloseBracket | Key::CloseCurlyBracket => KeyCode::BracketRight,
        Key::IntlBackslash => KeyCode::IntlBackslash,

        // Digits
        Key::Num0 => KeyCode::Digit0,
        Key::Num1 | Key::Exclamationmark => KeyCode::Digit1,
        Key::Num2 => KeyCode::Digit2,
        Key::Num3 => KeyCode::Digit3,
        Key::Num4 => KeyCode::Digit4,
        Key::Num5 => KeyCode::Digit5,
        Key::Num6 => KeyCode::Digit6,
        Key::Num7 => KeyCode::Digit7,
        Key::Num8 => KeyCode::Digit8,
        Key::Num9 => KeyCode::Digit9,

        // Letters
        Key::A => KeyCode::KeyA,
        Key::B => KeyCode::KeyB,
        Key::C => KeyCode::KeyC,
        Key::D => KeyCode::KeyD,
        Key::E => KeyCode::KeyE,
        Key::F => KeyCode::KeyF,
        Key::G => KeyCode::KeyG,
        Key::H => KeyCode::KeyH,
        Key::I => KeyCode::KeyI,
        Key::J => KeyCode::KeyJ,
        Key::K => KeyCode::KeyK,
        Key::L => KeyCode::KeyL,
        Key::M => KeyCode::KeyM,
        Key::N => KeyCode::KeyN,
        Key::O => KeyCode::KeyO,
        Key::P => KeyCode::KeyP,
        Key::Q => KeyCode::KeyQ,
        Key::R => KeyCode::KeyR,
        Key::S => KeyCode::KeyS,
        Key::T => KeyCode::KeyT,
        Key::U => KeyCode::KeyU,
        Key::V => KeyCode::KeyV,
        Key::W => KeyCode::KeyW,
        Key::X => KeyCode::KeyX,
        Key::Y => KeyCode::KeyY,
        Key::Z => KeyCode::KeyZ,

        // Function keys
        Key::F1 => KeyCode::F1,
        Key::F2 => KeyCode::F2,
        Key::F3 => KeyCode::F3,
        Key::F4 => KeyCode::F4,
        Key::F5 => KeyCode::F5,
        Key::F6 => KeyCode::F6,
        Key::F7 => KeyCode::F7,
        Key::F8 => KeyCode::F8,
        Key::F9 => KeyCode::F9,
        Key::F10 => KeyCode::F10,
        Key::F11 => KeyCode::F11,
        Key::F12 => KeyCode::F12,
        Key::F13 => KeyCode::F13,
        Key::F14 => KeyCode::F14,
        Key::F15 => KeyCode::F15,
        Key::F16 => KeyCode::F16,
        Key::F17 => KeyCode::F17,
        Key::F18 => KeyCode::F18,
        Key::F19 => KeyCode::F19,
        Key::F20 => KeyCode::F20,
        Key::F21 => KeyCode::F21,
        Key::F22 => KeyCode::F22,
        Key::F23 => KeyCode::F23,
        Key::F24 => KeyCode::F24,
        Key::F25 => KeyCode::F25,
        Key::F26 => KeyCode::F26,
        Key::F27 => KeyCode::F27,
        Key::F28 => KeyCode::F28,
        Key::F29 => KeyCode::F29,
        Key::F30 => KeyCode::F30,
        Key::F31 => KeyCode::F31,
        Key::F32 => KeyCode::F32,
        Key::F33 => KeyCode::F33,
        Key::F34 => KeyCode::F34,
        Key::F35 => KeyCode::F35,

        // Browser, media and launcher keys
        Key::BrowserBack => KeyCode::BrowserBack,
        Key::BrowserForward => KeyCode::BrowserForward,
        Key::BrowserRefresh => KeyCode::BrowserRefresh,
        Key::BrowserSearch => KeyCode::BrowserSearch,
        Key::BrowserHome => KeyCode::BrowserHome,
        Key::BrowserFavorites => KeyCode::BrowserFavorites,
        Key::BrowserStop => KeyCode::BrowserStop,
        Key::MediaPlayPause => KeyCode::MediaPlayPause,
        Key::MediaTrackNext => KeyCode::MediaTrackNext,
        Key::MediaTrackPrevious => KeyCode::MediaTrackPrevious,
        Key::MediaStop => KeyCode::MediaStop,
        Key::AudioVolumeMute => KeyCode::AudioVolumeMute,
        Key::AudioVolumeDown => KeyCode::AudioVolumeDown,
        Key::AudioVolumeUp => KeyCode::AudioVolumeUp,
        Key::LaunchMail => KeyCode::LaunchMail,
        Key::LaunchApp1 => KeyCode::LaunchApp1,
        Key::LaunchApp2 => KeyCode::LaunchApp2,

        // Modifiers
        Key::ShiftLeft => KeyCode::ShiftLeft,
        Key::ShiftRight => KeyCode::ShiftRight,
        Key::ControlLeft => KeyCode::ControlLeft,
        Key::ControlRight => KeyCode::ControlRight,
        Key::AltLeft => KeyCode::AltLeft,
        Key::AltRight => KeyCode::AltRight,
        Key::SuperLeft => KeyCode::SuperLeft,
        Key::SuperRight => KeyCode::SuperRight,

        // Locks and system keys
        Key::CapsLock => KeyCode::CapsLock,
        Key::NumLock => KeyCode::NumLock,
        Key::ScrollLock => KeyCode::ScrollLock,
        Key::PrintScreen => KeyCode::PrintScreen,
        Key::Pause => KeyCode::Pause,
        Key::Menu => KeyCode::ContextMenu,
        Key::Fn => KeyCode::Fn,
        Key::Eject => KeyCode::Eject,
        Key::Help => KeyCode::Help,
        Key::Power => KeyCode::Power,
        Key::Sleep => KeyCode::Sleep,
        Key::Clear => KeyCode::NumpadClear,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_key_codes() {
        assert_eq!(Key::A.windows_key_code(), Some(0x41));
        assert_eq!(Key::Num7.windows_key_code(), Some(0x37));
        assert_eq!(Key::F1.windows_key_code(), Some(0x70));
        assert_eq!(Key::F24.windows_key_code(), Some(0x87));
        assert_eq!(Key::F25.windows_key_code(), None);
        assert_eq!(Key::Questionmark.windows_key_code(), Some(0xBF));
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
        assert_eq!(keys.map(Key::native_key_code), expected);
    }

    #[test]
    fn mac_function_chars() {
        assert_eq!(Key::F1.mac_function_char(), Some(0xF704));
        assert_eq!(Key::A.mac_function_char(), None);
    }
}
