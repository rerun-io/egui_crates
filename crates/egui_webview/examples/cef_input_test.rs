//! Types into a page that CEF renders, by injecting events into egui's input, and checks the
//! values of the page's form fields after each step.
//!
//! A plugin adds the events to egui's input, as an integration would. The OS sees none of them,
//! and the window opens inactive, so the test can run while you use the computer.
//!
//! Prints `PASS` and exits with code 0 if the page reports the expected values after every step.
//!
//! ```sh
//! cargo run -p egui_webview --features cef --example cef_input_test
//! ```

#![expect(clippy::print_stdout, clippy::unwrap_used)] // A test.

use std::collections::VecDeque;
use std::sync::Arc;

use egui::mutex::Mutex;
use egui::{CentralPanel, Event, ImeEvent, Key, Modifiers, Pos2, vec2};
use egui_webview::{WebViewEvent, WebViewInstance, WebViewSource};

use common::{Injector, click, key, key_with_text};

mod common;

/// A text field, a `<select>` and a text area. On load and on every change, the page posts their
/// values as `field|select|area`, with the text area's newlines replaced by `/`.
const PAGE: &str = r#"<input id="field" style="position: fixed; left: 0; top: 0; width: 100%; height: 60px; font-size: 24px">
<select id="choice" style="position: fixed; left: 0; top: 100px; width: 200px; height: 40px">
    <option>a</option><option>b</option><option>c</option>
</select>
<textarea id="area" style="position: fixed; left: 0; top: 160px; width: 200px; height: 80px"></textarea>
<script>
    const report = () => window.ipc.postMessage(
        field.value + "|" + choice.value + "|" + area.value.replaceAll("\n", "/"));
    field.addEventListener("input", report);
    choice.addEventListener("change", report);
    area.addEventListener("input", report);
    window.addEventListener("load", report);
</script>"#;

/// Time to give the page after each step, in seconds.
const STEP_TIME: f64 = 0.7;

/// The steps for a page whose top-left corner is at `page`: each step's events, one entry per
/// pass, and the report the page should send after it.
fn steps(page: Pos2) -> Vec<(Vec<Vec<Event>>, &'static str)> {
    let field = page + vec2(100.0, 30.0);
    let choice = page + vec2(100.0, 120.0);
    let area = page + vec2(100.0, 200.0);
    let command = if cfg!(target_os = "macos") {
        Modifiers::MAC_CMD
    } else {
        Modifiers::CTRL
    };
    vec![
        (click(field), "|a|"),
        (typed("abcde"), "abcde|a|"),
        (key(Key::Backspace, Modifiers::NONE), "abcd|a|"),
        (
            [key(Key::ArrowLeft, Modifiers::NONE), typed("X")].concat(),
            "abcXd|a|",
        ),
        ([key(Key::A, command), typed("Z")].concat(), "Z|a|"),
        // Click the `<select>` to open its popup, then pick the next option with the arrow key
        // and Enter.
        (click(choice), "Z|a|"),
        (
            [
                key(Key::ArrowDown, Modifiers::NONE),
                key(Key::Enter, Modifiers::NONE),
            ]
            .concat(),
            "Z|b|",
        ),
        // Each Enter in the text area inserts exactly one newline.
        (
            [
                click(area),
                typed("a"),
                key(Key::Enter, Modifiers::NONE),
                typed("b"),
            ]
            .concat(),
            "Z|b|a/b",
        ),
        // Text committed by an input method arrives as `ImeEvent::Commit` without an
        // `Event::Text`, as on macOS while the IME is enabled for the page's field.
        (
            [
                key(Key::Q, Modifiers::NONE),
                vec![vec![ime(ImeEvent::Commit("q".to_owned()))]],
            ]
            .concat(),
            "Z|b|a/bq",
        ),
        (
            vec![
                vec![ime(ImeEvent::Preedit {
                    text: "ni".to_owned(),
                    active_range_chars: None,
                })],
                vec![ime(ImeEvent::Commit("你".to_owned()))],
            ],
            "Z|b|a/bq你",
        ),
    ]
}

fn ime(event: ImeEvent) -> Event {
    Event::Ime(event)
}

/// Key presses that type `text`: one press and release per letter or digit, with Shift for
/// uppercase letters.
fn typed(text: &str) -> Vec<Vec<Event>> {
    text.chars()
        .flat_map(|c| {
            let key = Key::from_name(&c.to_ascii_uppercase().to_string()).unwrap();
            let modifiers = if c.is_uppercase() {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
            key_with_text(key, modifiers, Some(&c.to_string()))
        })
        .collect()
}

/// The test: a page in the central panel, and the steps of [`steps`] run one after the other.
struct Test {
    queue: Arc<Mutex<VecDeque<Vec<Event>>>>,
    webview: Option<WebViewInstance>,
    /// The page's last report: the value of its text field.
    value: Option<String>,
    step: usize,
    step_started: Option<f64>,
    failures: Vec<usize>,
}

impl eframe::App for Test {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Self {
            queue,
            webview,
            value,
            step,
            step_started,
            failures,
        } = self;
        let ctx = ui.ctx().clone();
        let webview = webview.get_or_insert_with(|| {
            ctx.add_plugin(Injector(Arc::clone(queue)));
            WebViewInstance::new_cef(
                &ctx,
                egui::Id::unique("page"),
                &WebViewSource::Html(PAGE.to_owned()),
            )
            .unwrap()
        });
        let page = CentralPanel::default()
            .show(ui, |ui| {
                let response = webview.ui(ui, ui.available_size());
                for event in response.events {
                    if let WebViewEvent::Message(text) = event {
                        *value = Some(text);
                    }
                }
                response.response.rect.min
            })
            .inner;
        ctx.request_repaint();

        // Wait for the page's first report, which it sends on load.
        let Some(current) = value else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let steps = steps(page);
        match *step_started {
            None => {
                queue.lock().extend(steps[*step].0.iter().cloned());
                *step_started = Some(now);
            }
            Some(started) if now - started > STEP_TIME => {
                let expected = steps[*step].1;
                println!("step {step}: expected {expected:?}, got {current:?}");
                if current != expected {
                    failures.push(*step);
                }
                *step += 1;
                *step_started = None;
                if *step == steps.len() {
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
        if let Some(webview) = &mut self.webview {
            webview.place(ctx, &output.platform_output.native_views);
        }
    }
}

fn main() -> eframe::Result {
    if let Some(code) = egui_webview::run_cef_subprocess() {
        std::process::exit(code);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([500.0, 300.0])
            .with_active(false),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "cef_input_test",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Test {
                queue: Arc::new(Mutex::new(VecDeque::new())),
                webview: None,
                value: None,
                step: 0,
                step_started: None,
                failures: Vec::new(),
            }))
        }),
    )
}
