//! Moves the keyboard focus between egui widgets and a page with Tab, Shift-Tab and clicks, and
//! checks after each step which egui widget and which page element have the focus.
//!
//! Keys meant for egui are injected into egui's input, as in `cef_input_test`. A page in an
//! embedded view (`WKWebView`, `WebView2`) receives its keys and clicks from the OS rather than
//! from egui.
//! For those, the test brings the window to the front and sends OS events: `NSEvent`s through
//! `NSWindow::sendEvent` on macOS, and `SendInput` on Windows, which moves the mouse cursor.
//! With CEF, every event is injected into egui's input.
//!
//! Prints `PASS` and exits with code 0 if the focus is as expected after every step.
//!
//! ```sh
//! cargo run -p egui_webview --example focus_test
//! EGUI_WEBVIEW_BACKEND=cef cargo run -p egui_webview --features cef --example focus_test
//! ```

#![expect(clippy::print_stdout, clippy::unwrap_used)] // A test.

use std::sync::Arc;

use egui::{Event, Id, Key, Modifiers, vec2};
use egui_webview::{WebViewEvent, WebViewInstance, WebViewSource};

use synthetic::{Injector, Queue, click};

mod synthetic;

/// Two text fields. Whenever the focus changes, the page posts the id of the focused element:
/// `body` if neither field has the focus, or an empty string if the document doesn't have it.
const PAGE: &str = r#"<input id="a" style="display: block; margin: 8px">
<input id="b" style="display: block; margin: 8px">
<script>
    let last = null;
    setInterval(() => {
        const now = document.hasFocus() ? (document.activeElement?.id || "body") : "";
        if (now !== last) {
            last = now;
            window.ipc.postMessage(now);
        }
    }, 30);
</script>"#;

/// Time to give the page after each step, in seconds.
const STEP_TIME: f64 = 0.8;

/// Where egui's keyboard focus is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Egui {
    Before,
    Page,
    After,
    /// On no widget, or on a widget other than the above.
    Elsewhere,
}

/// What a step does.
#[derive(Clone, Copy, Debug)]
enum Action {
    /// Focus the button before the page.
    FocusBefore,
    /// Press Tab (or Shift-Tab) while an egui widget has the keyboard focus.
    EguiTab { shift: bool },
    /// Press Tab (or Shift-Tab) while the page has the keyboard focus.
    PageTab { shift: bool },
    /// Click the second text field of the page.
    ClickPage,
    /// Click the button after the page.
    ClickAfter,
}

/// Each step, with the expected focus after it: the page's report (see [`PAGE`]) and egui's
/// focused widget.
const STEPS: &[(Action, &str, Egui)] = &[
    (Action::FocusBefore, "", Egui::Before),
    (Action::EguiTab { shift: false }, "a", Egui::Page),
    (Action::PageTab { shift: false }, "b", Egui::Page),
    (Action::PageTab { shift: false }, "", Egui::After),
    (Action::EguiTab { shift: true }, "b", Egui::Page),
    (Action::PageTab { shift: true }, "a", Egui::Page),
    (Action::PageTab { shift: true }, "", Egui::Before),
    (Action::ClickPage, "b", Egui::Page),
    (Action::ClickAfter, "", Egui::Elsewhere),
];

/// A point inside the page's second text field, relative to the page's top-left corner.
const FIELD_B: egui::Vec2 = vec2(40.0, 45.0);

/// Tab, with Shift if `shift`.
fn tab(shift: bool) -> Vec<Vec<Event>> {
    let modifiers = if shift {
        Modifiers::SHIFT
    } else {
        Modifiers::NONE
    };
    synthetic::key(Key::Tab, modifiers)
}

/// The test: a page between two buttons, and the steps of [`STEPS`] run one after the other.
struct Test {
    /// Whether every event is injected into egui's input, because a CEF page receives its input
    /// through egui.
    cef: bool,
    queue: Queue,
    webview: WebViewInstance,
    /// The page's last report, see [`PAGE`].
    page_focus: Option<String>,
    step: usize,
    step_started: Option<f64>,
    failures: Vec<usize>,
}

impl eframe::App for Test {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Self {
            cef,
            queue,
            webview,
            page_focus,
            step,
            step_started,
            failures,
        } = self;
        let cef = *cef;
        let ctx = ui.ctx().clone();
        let (before, page, after) = egui::CentralPanel::default()
            .show(ui, |ui| {
                let before = ui.button("Before");
                let page = webview.ui(ui, vec2(200.0, 100.0));
                for event in page.events {
                    if let WebViewEvent::Message(focus) = event {
                        *page_focus = Some(focus);
                    }
                }
                let after = ui.button("After");
                (before, page.response, after)
            })
            .inner;
        ctx.request_repaint();

        // Wait for the page's first report, which shows that it has loaded.
        let Some(current_page) = page_focus else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let (action, expected_page, expected_egui) = STEPS[*step];
        match *step_started {
            None => {
                match action {
                    Action::FocusBefore => before.request_focus(),
                    Action::EguiTab { shift } => queue.lock().extend(tab(shift)),
                    Action::PageTab { shift } if cef => queue.lock().extend(tab(shift)),
                    Action::PageTab { shift } => native::press_tab(frame, shift),
                    Action::ClickPage if cef => queue.lock().extend(click(page.rect.min + FIELD_B)),
                    Action::ClickPage => native::click(frame, &ctx, page.rect.min + FIELD_B),
                    Action::ClickAfter => queue.lock().extend(click(after.rect.center())),
                }
                *step_started = Some(now);
            }
            Some(started) if now - started > STEP_TIME => {
                let egui_focus = match ctx.memory(|mem| mem.focused()) {
                    Some(id) if id == before.id => Egui::Before,
                    Some(id) if id == page.id => Egui::Page,
                    Some(id) if id == after.id => Egui::After,
                    _ => Egui::Elsewhere,
                };
                let ok = current_page == expected_page && egui_focus == expected_egui;
                println!(
                    "step {step} {action:?}: expected {expected_page:?} {expected_egui:?}, \
                     got {current_page:?} {egui_focus:?}{}",
                    if ok { "" } else { "  <-- FAIL" }
                );
                if !ok {
                    failures.push(*step);
                }
                *step += 1;
                *step_started = None;
                if *step == STEPS.len() {
                    if failures.is_empty() {
                        println!("PASS");
                        std::process::exit(0);
                    }
                    println!("FAIL: steps {failures:?}");
                    std::process::exit(1);
                }
            }
            Some(_) => {}
        }
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webview
            .place(ctx, &output.platform_output.embedded_views);
    }
}

fn main() -> eframe::Result {
    #[cfg(feature = "cef")]
    if let Some(code) = egui_webview::run_cef_subprocess() {
        std::process::exit(code);
    }
    let cef = cfg!(feature = "cef")
        && (cfg!(target_os = "linux")
            || std::env::var("EGUI_WEBVIEW_BACKEND").is_ok_and(|backend| backend == "cef"));

    let viewport = egui::ViewportBuilder::default().with_inner_size([400.0, 300.0]);
    // On Windows, a page only shows through egui's surface in a transparent window.
    #[cfg(target_os = "windows")]
    let viewport = viewport.with_transparent(true);

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        #[cfg(target_os = "windows")]
        wgpu_options: {
            let mut options = egui_wgpu::WgpuConfiguration::default();
            egui_webview::wgpu_configuration(&mut options);
            options
        },
        ..Default::default()
    };
    eframe::run_native(
        "focus_test",
        options,
        Box::new(move |cc| {
            let queue = Queue::default();
            cc.egui_ctx.add_plugin(Injector(Arc::clone(&queue)));
            let webview = WebViewInstance::new(
                &cc.egui_ctx,
                Id::unique("page"),
                &WebViewSource::Html(PAGE.to_owned()),
                cc.embedded_views(),
            )?;
            Ok(Box::new(Test {
                cef,
                queue,
                webview,
                page_focus: None,
                step: 0,
                step_started: None,
                failures: Vec::new(),
            }))
        }),
    )
}

/// Key presses and clicks sent through `AppKit`, for a page in an embedded view.
#[cfg(target_os = "macos")]
mod native {
    #![expect(unsafe_code)]

    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::{NSPoint, NSString};
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

    fn window(frame: &eframe::Frame) -> objc2::rc::Retained<objc2_app_kit::NSWindow> {
        let RawWindowHandle::AppKit(handle) = frame.window_handle().unwrap().as_raw() else {
            panic!("Not an AppKit window");
        };
        // SAFETY: the handle points to winit's `NSView`, which lives as long as the window.
        let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let window = view.window().unwrap();
        window.makeKeyAndOrderFront(None);
        window
    }

    pub fn press_tab(frame: &eframe::Frame, shift: bool) {
        let window = window(frame);
        let (flags, characters) = if shift {
            (NSEventModifierFlags::Shift, "\u{19}")
        } else {
            (NSEventModifierFlags::empty(), "\t")
        };
        let characters = NSString::from_str(characters);
        for kind in [NSEventType::KeyDown, NSEventType::KeyUp] {
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                kind,
                NSPoint::ZERO,
                flags,
                0.0,
                window.windowNumber(),
                None,
                &characters,
                &characters,
                false,
                48, // `kVK_Tab`
            )
            .unwrap();
            window.sendEvent(&event);
        }
    }

    pub fn click(frame: &eframe::Frame, _ctx: &egui::Context, pos: egui::Pos2) {
        let window = window(frame);
        // `AppKit` window coordinates have their origin at the bottom left, egui's at the top left.
        let height = window.contentLayoutRect().size.height;
        let location = NSPoint::new(f64::from(pos.x), height - f64::from(pos.y));
        for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
            let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                kind,
                location,
                NSEventModifierFlags::empty(),
                0.0,
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .unwrap();
            window.sendEvent(&event);
        }
    }
}

/// Key presses and clicks sent through the OS input queue, for a page in an embedded view.
#[cfg(target_os = "windows")]
mod native {
    #![expect(unsafe_code)]
    #![expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)] // Win32 takes `i32` coordinates and sizes.

    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        KEYEVENTF_KEYUP, MOUSE_EVENT_FLAGS, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
        SendInput, VIRTUAL_KEY, VK_SHIFT, VK_TAB,
    };
    use windows::Win32::UI::WindowsAndMessaging::{SetCursorPos, SetForegroundWindow};

    fn to_front(frame: &eframe::Frame) {
        let RawWindowHandle::Win32(handle) = frame.window_handle().unwrap().as_raw() else {
            panic!("Not a Win32 window");
        };
        // SAFETY: the handle is eframe's live window.
        unsafe { SetForegroundWindow(HWND(handle.hwnd.get() as _)) }
            .ok()
            .ok();
    }

    fn key(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        }
    }

    fn mouse(flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        // SAFETY: `inputs` is a valid slice, and the size passed is that of `INPUT`.
        unsafe { SendInput(inputs, size_of::<INPUT>() as i32) };
    }

    pub fn press_tab(frame: &eframe::Frame, shift: bool) {
        to_front(frame);
        let none = KEYBD_EVENT_FLAGS(0);
        let mut inputs = vec![key(VK_TAB, none), key(VK_TAB, KEYEVENTF_KEYUP)];
        if shift {
            inputs.insert(0, key(VK_SHIFT, none));
            inputs.push(key(VK_SHIFT, KEYEVENTF_KEYUP));
        }
        send(&inputs);
    }

    pub fn click(frame: &eframe::Frame, ctx: &egui::Context, pos: egui::Pos2) {
        to_front(frame);
        let (inner, ppp) = ctx.input(|i| (i.viewport().inner_rect, i.pixels_per_point));
        let screen = (inner.unwrap().min + pos.to_vec2()) * ppp;
        // SAFETY: `SetCursorPos` only takes screen coordinates.
        unsafe { SetCursorPos(screen.x as i32, screen.y as i32) }.unwrap();
        send(&[mouse(MOUSEEVENTF_LEFTDOWN), mouse(MOUSEEVENTF_LEFTUP)]);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod native {
    pub fn press_tab(_frame: &eframe::Frame, _shift: bool) {
        unreachable!("Only CEF here");
    }

    pub fn click(_frame: &eframe::Frame, _ctx: &egui::Context, _pos: egui::Pos2) {
        unreachable!("Only CEF here");
    }
}
