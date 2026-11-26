#![expect(clippy::print_stdout)] // A benchmark.

use std::time::Instant;

use egui::mutex::Mutex;
use egui_webview::{CefOptions, WebViewInstance, WebViewSource};

/// How fast the page's height changes, in points per second.
const SPEED: f32 = 300.0;

/// The page's height moves between these, in points.
const HEIGHT_RANGE: [f32; 2] = [200.0, 500.0];

/// Time to let the page load before the measurement starts, in seconds.
const WARMUP: f32 = 2.0;

/// How long the measurement runs, in seconds.
const DURATION: f32 = 6.0;

/// The target of the CEF backend's trace logs about frames.
const FRAMES_LOG_TARGET: &str = "egui_webview::frames";

/// Three columns of text between a header and a footer, so that a change of height moves the
/// footer and reflows the text.
const PAGE: &str = r#"<!doctype html><html><head><style>
html, body { margin: 0; height: 100%; font-family: sans-serif }
body { display: flex; flex-direction: column }
header, footer { background: #246; color: #fff; padding: 8px }
main { flex: 1; overflow: auto; columns: 3; padding: 8px }
/*ANIMATION*/
</style></head><body><header>Header</header><main id="main"></main><footer>Footer</footer>
<script>
    let html = "";
    for (let i = 0; i < 200; i++) {
        html += "<p>Paragraph " + i + " lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>";
    }
    document.getElementById("main").innerHTML = html;
</script></body></html>"#;

/// Replaces `/*ANIMATION*/` in [`PAGE`].
const ANIMATION: &str = "@keyframes spin { to { transform: rotate(360deg) } } \
                         header { animation: spin 2s linear infinite }";

/// What the command line asks for.
struct Args {
    /// `CefOptions::frame_rate`, if not the default.
    fps: Option<u32>,

    /// Keep the page's size, and only repaint when something asks for it.
    still: bool,

    /// Run a CSS animation in the page.
    animated: bool,
}

impl Args {
    fn parse() -> Self {
        let mut args = Self {
            fps: None,
            still: false,
            animated: false,
        };
        let mut words = std::env::args().skip(1);
        while let Some(word) = words.next() {
            match word.as_str() {
                "--fps" => args.fps = words.next().and_then(|fps| fps.parse().ok()),
                "--still" => args.still = true,
                "--animated" => args.animated = true,
                unknown => println!("Ignoring unknown argument {unknown:?}"),
            }
        }
        args
    }
}

/// What the measurement counted.
#[derive(Default)]
struct Stats {
    /// When the measurement started. Nothing is counted before.
    start: Option<Instant>,

    /// For each pass that showed the page: how many points the frame's height was off.
    behind: Vec<f32>,

    /// The frames that CEF painted with the size it was last asked for.
    kept_frames: usize,

    /// The frames that CEF painted with an older size, which the backend drops.
    stale_frames: usize,
}

/// Collects the CEF backend's trace logs about frames into [`Stats`].
#[derive(Default)]
struct FrameLog {
    stats: Mutex<Stats>,
}

impl log::Log for FrameLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == FRAMES_LOG_TARGET
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut stats = self.stats.lock();
        if stats.start.is_none() {
            return;
        }
        let message = record.args().to_string();
        let words: Vec<&str> = message.split(' ').collect();
        match words.as_slice() {
            ["shown", widget, frame] => {
                if let (Ok(widget), Ok(frame)) = (widget.parse::<f32>(), frame.parse::<f32>()) {
                    stats.behind.push((widget - frame).abs());
                }
            }
            ["painted", _, "kept"] => stats.kept_frames += 1,
            ["painted", _, "stale"] => stats.stale_frames += 1,
            _ => {}
        }
    }

    fn flush(&self) {}
}

impl Stats {
    fn print(&mut self) {
        let Some(start) = self.start else {
            println!("The measurement never started");
            return;
        };
        let Self {
            behind,
            kept_frames,
            stale_frames,
            ..
        } = self;
        let seconds = start.elapsed().as_secs_f32();
        let passes = behind.len();
        println!("Passes that showed the page: {:.0} per second", passes as f32 / seconds);
        println!(
            "Frames that CEF painted: {:.0} per second, and {stale_frames} of {} had a stale size",
            (*kept_frames + *stale_frames) as f32 / seconds,
            *kept_frames + *stale_frames,
        );
        if passes == 0 {
            return;
        }
        behind.sort_by(f32::total_cmp);
        let mean = behind.iter().sum::<f32>() / passes as f32;
        let p95 = behind[(passes * 95 / 100).min(passes - 1)];
        let ms = |points: f32| 1e3 * points / SPEED;
        println!(
            "Lag of the shown frame: mean {mean:.1} points ({:.0} ms), 95th percentile {p95:.0} \
             points ({:.0} ms)",
            ms(mean),
            ms(p95),
        );
    }
}

struct App {
    webview: WebViewInstance,
    frame_log: &'static FrameLog,
    still: bool,
    start: Instant,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let Self {
            webview,
            frame_log,
            still,
            start,
        } = self;
        let [min, max] = HEIGHT_RANGE;

        let measured = start.elapsed().as_secs_f32() - WARMUP;
        if DURATION < measured {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else if 0.0 <= measured {
            frame_log.stats.lock().start.get_or_insert_with(Instant::now);
        }

        let height = if *still || measured < 0.0 {
            min
        } else {
            // A triangle wave.
            let span = max - min;
            let phase = (measured * SPEED) % (2.0 * span);
            min + if phase < span {
                phase
            } else {
                2.0 * span - phase
            }
        };

        egui::CentralPanel::default().show(ui, |ui| {
            webview.ui(ui, egui::vec2(ui.available_width(), height));
        });

        if *still {
            // Wake up to start and to end the measurement.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f32(WARMUP.max(DURATION)));
        } else {
            ui.ctx().request_repaint();
        }
    }

    fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
        self.webview
            .place(ctx, &output.platform_output.embedded_views);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.frame_log.stats.lock().print();
    }
}

pub fn run() -> eframe::Result {
    // Start the webview runtime before anything else. With CEF, Chromium
    // runs its helper processes (renderer, GPU, …) from this same executable. In such a
    // process, this call runs the helper and then exits, so nothing below runs there.
    let _runtime = egui_webview::Runtime::new();

    let args = Args::parse();

    let frame_log: &'static FrameLog = Box::leak(Box::default());
    if log::set_logger(frame_log).is_ok() {
        log::set_max_level(log::LevelFilter::Trace);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "egui_webview resize_bench",
        options,
        Box::new(move |cc| {
            let options = CefOptions {
                frame_rate: args.fps,
                ..Default::default()
            };
            let animation = if args.animated { ANIMATION } else { "" };
            let webview = WebViewInstance::new_cef_with_options(
                &cc.egui_ctx,
                egui::Id::unique("page"),
                &WebViewSource::Html(PAGE.replace("/*ANIMATION*/", animation)),
                options,
            )?;
            Ok(Box::new(App {
                webview,
                frame_log,
                still: args.still,
                start: Instant::now(),
            }))
        }),
    )
}
