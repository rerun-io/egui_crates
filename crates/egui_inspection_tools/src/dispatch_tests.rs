//! [`crate::call`] against a fake app, to pin how the tools talk to the [`Transport`].

use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use accesskit::{Node, NodeId, Rect, Role, TreeId, TreeInfo, TreeUpdate};
use egui::Event;
use egui_inspection::protocol::{EncodedPng, Request, Response};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::{BoxFuture, Content, Effect, ToolOutput, Transport, call, tool, tools};

/// The app's scale, so a test sees physical `AccessKit` bounds turned into logical points.
const PIXELS_PER_POINT: f32 = 2.0;

/// A fake egui app: a window holding one `Save` button. It records every request, and answers
/// `ApplyEvents` with whatever `on_events` says.
struct FakeApp {
    requests: Mutex<Vec<Request>>,
    step: AtomicU64,
    on_events: fn(&[Event]) -> Response,
}

impl FakeApp {
    fn new() -> Self {
        Self::answering_events_with(|_| Response::Done)
    }

    fn answering_events_with(on_events: fn(&[Event]) -> Response) -> Self {
        Self {
            requests: Mutex::default(),
            step: AtomicU64::new(10),
            on_events,
        }
    }

    fn requests(&self) -> Vec<Request> {
        self.requests.lock().clone()
    }

    fn tree_fetches(&self) -> usize {
        self.requests()
            .iter()
            .filter(|req| matches!(req, Request::GetTree))
            .count()
    }

    fn tree() -> TreeUpdate {
        let mut window = Node::new(Role::Window);
        window.set_children(vec![NodeId(2)]);
        let mut save = Node::new(Role::Button);
        save.set_label("Save");
        save.set_bounds(Rect::new(20.0, 40.0, 60.0, 80.0));
        TreeUpdate {
            nodes: vec![(NodeId(1), window), (NodeId(2), save)],
            tree: Some(TreeInfo::new(NodeId(1))),
            tree_id: TreeId::ROOT,
            focus: NodeId(1),
        }
    }
}

impl Transport for FakeApp {
    fn request(&self, req: Request) -> BoxFuture<'_, Result<Response, String>> {
        let response = match &req {
            Request::GetTree => Response::Tree {
                step: self.step.fetch_add(1, Ordering::Relaxed),
                pixels_per_point: PIXELS_PER_POINT,
                accesskit: Some(Self::tree()),
            },
            Request::GetScreenshot { .. } => Response::Screenshot(EncodedPng {
                size: [4, 3],
                bytes: b"png".to_vec(),
            }),
            Request::ApplyEvents { events } => (self.on_events)(events),
            _ => Response::Done,
        };
        self.requests.lock().push(req);
        Box::pin(std::future::ready(Ok(response)))
    }
}

/// A transport whose every request fails.
struct Unreachable;

impl Transport for Unreachable {
    fn request(&self, _req: Request) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(std::future::ready(Err("connection reset".to_owned())))
    }
}

/// A timer that has already gone off, however long it was asked to wait.
fn instant_sleep(_: Duration) -> BoxFuture<'static, ()> {
    Box::pin(std::future::ready(()))
}

/// A timer whose short waits end at once and whose long ones never do: `wait_for` polls freely
/// and never times out.
fn sleep_forever_on_long_waits(duration: Duration) -> BoxFuture<'static, ()> {
    if duration < Duration::from_secs(1) {
        Box::pin(std::future::ready(()))
    } else {
        Box::pin(std::future::pending())
    }
}

/// Run a future whose waits all end at once. Fails rather than hangs if one doesn't.
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..10_000 {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
    panic!("the future never finished");
}

fn run(app: &dyn Transport, name: &str, args: &Value) -> ToolOutput {
    let args = args.as_object().expect("an object").clone();
    block_on(call(app, &instant_sleep, name, args))
}

fn text(output: &ToolOutput) -> &str {
    match output.content.first() {
        Some(Content::Text(text)) => text,
        other => panic!("expected text first, got {other:?}"),
    }
}

#[test]
fn every_tool_is_dispatched_and_takes_an_object() {
    let mut names: Vec<_> = tools().iter().map(|def| def.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), tools().len(), "tool names are unique");

    for def in tools() {
        assert_eq!(def.input_schema["type"], "object", "{}", def.name);
        let output = run(&FakeApp::new(), def.name, &json!({}));
        assert!(
            !text(&output).starts_with("unknown tool"),
            "`{}` is listed but not dispatched",
            def.name
        );
    }
}

#[test]
fn only_the_observing_tools_are_read_only() {
    let read_only: Vec<_> = tools()
        .iter()
        .filter(|def| def.effect.is_read_only())
        .map(|def| def.name)
        .collect();
    assert_eq!(
        read_only,
        ["widget_tree", "get_widget", "screenshot", "wait_for"]
    );
    assert_eq!(
        tool("batch").map(|def| def.effect),
        Some(Effect::Destructive)
    );
}

#[test]
fn an_unknown_tool_is_an_error_and_reaches_nothing() {
    let app = FakeApp::new();
    let output = run(&app, "launch_missiles", &json!({}));
    assert!(output.is_error);
    assert_eq!(text(&output), "unknown tool `launch_missiles`");
    assert!(app.requests().is_empty());
}

#[test]
fn a_misspelled_argument_is_rejected_before_reaching_the_app() {
    let app = FakeApp::new();
    let output = run(&app, "click", &json!({ "content_contain": "Save" }));
    assert!(output.is_error);
    assert!(
        text(&output).starts_with("unknown argument `content_contain`"),
        "{output:?}"
    );
    assert!(app.requests().is_empty());
}

#[test]
fn a_wrongly_typed_argument_is_an_error() {
    let output = run(
        &FakeApp::new(),
        "resize",
        &json!({ "width": "wide", "height": 1 }),
    );
    assert!(output.is_error);
    assert!(
        text(&output).starts_with("failed to deserialize parameters:"),
        "{output:?}"
    );
}

#[test]
fn a_transport_failure_is_an_error_result() {
    let output = run(&Unreachable, "widget_tree", &json!({}));
    assert!(output.is_error);
    assert_eq!(text(&output), "connection reset");
}

#[test]
fn a_click_on_a_raw_pos_needs_no_tree() {
    let app = FakeApp::new();
    let output = run(&app, "click", &json!({ "pos": { "x": 1.0, "y": 2.0 } }));
    assert!(!output.is_error, "{output:?}");
    assert_eq!(app.tree_fetches(), 0);
}

#[test]
fn a_click_by_text_lands_on_the_center_in_logical_points() {
    let output = run(
        &FakeApp::new(),
        "click",
        &json!({ "content_contains": "save" }),
    );
    let result = output.structured.expect("a structured result");
    assert_eq!(result["clicked_id"], "2");
    // Physical bounds (20, 40)–(60, 80) at 2 pixels per point.
    assert_eq!(result["pos"], json!([20.0, 30.0]));
}

#[test]
fn modifiers_are_released_even_when_the_key_press_fails() {
    let app = FakeApp::answering_events_with(|events| match events {
        [Event::Key { .. }, ..] => Response::Error {
            message: "frame failed".to_owned(),
        },
        _ => Response::Done,
    });
    let output = run(
        &app,
        "press_key",
        &json!({ "key": "S", "modifiers": { "ctrl": true } }),
    );
    assert!(output.is_error);
    assert_eq!(text(&output), "frame failed");
    let Some(Request::ApplyEvents { events }) = app.requests().pop() else {
        panic!("the last request applies events");
    };
    assert!(
        matches!(events.as_slice(), [Event::ModifiersChanged(m)] if *m == egui::Modifiers::NONE),
        "{events:?}"
    );
}

#[test]
fn a_batch_stops_at_the_first_error_and_keeps_images_after_their_step() {
    let app = FakeApp::new();
    let output = run(
        &app,
        "batch",
        &json!({ "actions": [
            { "name": "screenshot" },
            { "name": "click", "args": {} },
            { "name": "resize", "args": { "width": 1, "height": 1 } },
        ] }),
    );
    assert!(output.is_error);
    let [
        Content::Text(first),
        Content::Image { mime_type, .. },
        Content::Text(second),
    ] = output.content.as_slice()
    else {
        panic!("one text per step, the image after its step: {output:?}");
    };
    assert_eq!(mime_type, "image/png");
    let first: Value = serde_json::from_str(first).expect("json");
    assert_eq!(first["name"], "screenshot");
    assert_eq!(first["isError"], false);
    let second: Value = serde_json::from_str(second).expect("json");
    assert_eq!(second["name"], "click");
    assert_eq!(second["isError"], true);
    assert!(
        !app.requests()
            .iter()
            .any(|req| matches!(req, Request::Resize { .. })),
        "the step after the failure never runs"
    );
}

#[test]
fn a_nested_batch_is_rejected_before_anything_runs() {
    let app = FakeApp::new();
    let output = run(
        &app,
        "batch",
        &json!({ "actions": [
            { "name": "screenshot" },
            { "name": "batch", "args": { "actions": [] } },
        ] }),
    );
    assert!(output.is_error);
    assert_eq!(text(&output), "nested `batch` is not allowed");
    assert!(app.requests().is_empty());
}

#[test]
fn wait_for_looks_once_before_a_timer_that_has_already_gone_off() {
    let app = FakeApp::new();
    let output = run(&app, "wait_for", &json!({ "content_contains": "Load" }));
    assert!(output.is_error);
    assert!(
        text(&output).starts_with("wait_for timed out"),
        "{output:?}"
    );
    assert_eq!(app.tree_fetches(), 1);
}

#[test]
fn wait_for_polls_until_enough_frames_have_passed() {
    let app = FakeApp::new();
    let args = json!({ "min_steps": 3 })
        .as_object()
        .cloned()
        .expect("object");
    let output = block_on(call(&app, &sleep_forever_on_long_waits, "wait_for", args));
    let result = output.structured.expect("a structured result");
    assert_eq!(result["steps_waited"], 3);
    assert_eq!(app.tree_fetches(), 4);
}

#[test]
fn wait_for_needs_something_to_wait_for() {
    let app = FakeApp::new();
    let output = run(&app, "wait_for", &json!({}));
    assert!(output.is_error);
    assert!(app.requests().is_empty());
}
