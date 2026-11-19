//! How the tools reach the egui app: one [`Request`] in, one [`Response`] out.
//!
//! The tools depend only on [`Transport`], so they don't care whether the bytes travel over a
//! TCP socket, a gRPC call, or an in-process channel to the app's own inspection handler.

use std::pin::Pin;
use std::time::Duration;

use egui_inspection::protocol::{EncodedPng, Request, Response};

cfg_select! {
    // The web is single-threaded, and its futures (timers, `fetch`, …) are not `Send`.
    target_arch = "wasm32" => {
        /// A boxed future, the return type of [`Transport::request`] and [`Sleep::sleep`].
        ///
        /// `Send` everywhere but on `wasm32`.
        pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

        /// `Send` everywhere but on `wasm32`.
        pub trait MaybeSend {}
        impl<T: ?Sized> MaybeSend for T {}

        /// `Sync` everywhere but on `wasm32`.
        pub trait MaybeSync {}
        impl<T: ?Sized> MaybeSync for T {}
    }
    _ => {
        /// A boxed future, the return type of [`Transport::request`] and [`Sleep::sleep`].
        ///
        /// `Send` everywhere but on `wasm32`.
        pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

        /// `Send` everywhere but on `wasm32`.
        pub trait MaybeSend: Send {}
        impl<T: ?Sized + Send> MaybeSend for T {}

        /// `Sync` everywhere but on `wasm32`.
        pub trait MaybeSync: Sync {}
        impl<T: ?Sized + Sync> MaybeSync for T {}
    }
}

/// Carries one [`Request`] to the egui app and returns its single [`Response`].
pub trait Transport: MaybeSend + MaybeSync {
    /// Send `req` and resolve with the matching [`Response`].
    ///
    /// Implementations must guarantee one response per request even under concurrent callers,
    /// and a request that has started must not be abandoned halfway: the tools never drop an
    /// in-flight request, so a framed byte stream stays in sync.
    fn request(&self, req: Request) -> BoxFuture<'_, Result<Response, String>>;
}

/// Waits for a while, using whatever timer the host has.
///
/// Only `wait_for` waits: between polls of the widget tree, and for its timeout.
/// Any `Fn(Duration) -> BoxFuture<'static, ()>` is a `Sleep`.
pub trait Sleep: MaybeSend + MaybeSync {
    /// Resolve after `duration`.
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()>;
}

impl<F> Sleep for F
where
    F: Fn(Duration) -> BoxFuture<'static, ()> + MaybeSend + MaybeSync,
{
    fn sleep(&self, duration: Duration) -> BoxFuture<'_, ()> {
        self(duration)
    }
}

/// A freshly-fetched `AccessKit` tree plus the geometry needed to map it to screen pixels.
pub struct TreeSnapshot {
    /// The app's frame counter when the tree was captured.
    pub step: u64,

    /// Physical pixels per logical point, to map the tree's physical bounds to logical points.
    pub pixels_per_point: f32,

    /// `None` until the app has produced its first `AccessKit` update.
    pub tree: Option<accesskit_consumer::Tree>,
}

/// Fetch the current `AccessKit` tree, built into a queryable [`accesskit_consumer::Tree`].
///
/// # Errors
/// On transport failure or an unexpected response.
pub async fn fetch_tree(transport: &dyn Transport) -> Result<TreeSnapshot, String> {
    match transport.request(Request::GetTree).await? {
        Response::Tree {
            step,
            pixels_per_point,
            accesskit,
        } => Ok(TreeSnapshot {
            step,
            pixels_per_point,
            tree: accesskit.map(|update| accesskit_consumer::Tree::new(update, false)),
        }),
        Response::Error { message } => Err(message),
        _ => Err("unexpected response to GetTree".to_owned()),
    }
}

/// Inject input events; resolves after the events have been processed by a frame.
///
/// # Errors
/// On transport failure or an unexpected response.
pub async fn apply_events(
    transport: &dyn Transport,
    events: Vec<egui::Event>,
) -> Result<(), String> {
    request_done(transport, Request::ApplyEvents { events }, "ApplyEvents").await
}

/// Resize the viewport (logical points).
///
/// # Errors
/// On transport failure or an unexpected response.
pub async fn resize(transport: &dyn Transport, width: u32, height: u32) -> Result<(), String> {
    request_done(transport, Request::Resize { width, height }, "Resize").await
}

/// Capture a screenshot, downscaled to `pixels_per_point` pixels per logical point
/// (`1.0` = logical size).
///
/// # Errors
/// On transport failure or an unexpected response.
pub async fn screenshot(
    transport: &dyn Transport,
    pixels_per_point: f32,
) -> Result<EncodedPng, String> {
    let req = Request::GetScreenshot {
        pixels_per_point: Some(pixels_per_point),
    };
    match transport.request(req).await? {
        Response::Screenshot(png) => Ok(png),
        Response::Error { message } => Err(message),
        _ => Err("unexpected response to GetScreenshot".to_owned()),
    }
}

/// Send a request that expects a bare [`Response::Done`]. `what` names the request for error
/// messages.
async fn request_done(transport: &dyn Transport, req: Request, what: &str) -> Result<(), String> {
    match transport.request(req).await? {
        Response::Done => Ok(()),
        Response::Error { message } => Err(message),
        _ => Err(format!("unexpected response to {what}")),
    }
}
