//! Moves the keyboard focus between egui widgets and a page with Tab, Shift-Tab and clicks, and
//! checks after each step which egui widget and which page element have the focus.
//!
//! Every event is injected into egui's input, as in `cef_input_test`, because a CEF page receives
//! its input through egui.
//!
//! Prints `PASS` and exits with code 0 if the focus is as expected after every step.
//!
//! ```sh
//! EGUI_WEBVIEW_BACKEND=cef cargo run -p egui_webview --features cef --example focus_test
//! ```

#![expect(clippy::print_stdout)] // A test.

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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([400.0, 300.0]),
        renderer: eframe::Renderer::Wgpu,
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

mod native {
    pub fn press_tab(_frame: &eframe::Frame, _shift: bool) {
        unreachable!("Only CEF here");
    }

    pub fn click(_frame: &eframe::Frame, _ctx: &egui::Context, _pos: egui::Pos2) {
        unreachable!("Only CEF here");
    }
}
