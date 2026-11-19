//! Bridge between the MCP server and a running egui app, speaking the request/response
//! [`egui_inspection`] protocol.
//!
//! Each MCP tool call turns into one or more [`Request`]s, each answered by exactly one
//! [`Response`]. How those messages travel is abstracted behind the [`Transport`] trait (from
//! [`egui_inspection_tools`]), so the tools don't care whether the bytes go over a framed TCP
//! stream or some host's own channel:
//!
//! - [`Bridge::connect`] dials a TCP `host:port` (with connect-retry), reads the protocol
//!   handshake, and drives a framed byte transport — what the standalone `egui-mcp` binary uses.
//! - [`Bridge::with_transport`] wraps any [`Transport`] implementation, so a host that already
//!   has a request/response channel to the app (e.g. `re_mcp` over a gRPC unary RPC) can drive
//!   the same tools without re-implementing the byte framing.

use std::time::Duration;

use egui_inspection::protocol::{
    EncodedPng, Request, Response, decode_frame_body, decode_frame_len, decode_handshake,
    encode_frame,
};
pub use egui_inspection_tools::{BoxFuture, Transport, TreeSnapshot};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::Mutex;

/// How long to keep retrying the initial TCP connect before giving up. Covers the case where
/// the user launches the app and attaches in quick succession.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Identity of the connected peer, captured at connect time.
#[derive(Debug, Clone, Serialize)]
pub struct PeerInfo {
    /// `host:port` for a TCP connection, or a description for a custom transport.
    pub transport: String,
    pub protocol_version: u32,
    pub label: Option<String>,
}

/// The reader + writer halves of a byte-stream connection, behind one lock so concurrent
/// requests don't interleave on the wire.
struct Conn {
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
}

/// The built-in [`Transport`]: length-prefixed `MessagePack` frames over any async byte stream
/// (a TCP socket for `egui-mcp`, but any `AsyncRead`/`AsyncWrite` pair works).
pub struct FramedTransport {
    conn: Mutex<Conn>,
}

impl FramedTransport {
    /// Wrap a reader/writer pair.
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self {
            conn: Mutex::new(Conn {
                reader: Box::new(reader),
                writer: Box::new(writer),
            }),
        }
    }
}

impl Transport for FramedTransport {
    fn request(&self, req: Request) -> BoxFuture<'_, Result<Response, String>> {
        Box::pin(async move {
            let mut conn = self.conn.lock().await;
            write_framed(&mut conn.writer, &req)
                .await
                .map_err(|e| e.to_string())?;
            read_framed(&mut conn.reader)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// A live connection to an egui inspection peer, driving it through some [`Transport`].
pub struct Bridge {
    transport: Box<dyn Transport>,
    pub peer_info: PeerInfo,
}

impl Bridge {
    /// Connect to a TCP `host:port`, retrying until `timeout` (default 10s) elapses, then read
    /// the protocol handshake and the peer's label.
    ///
    /// # Errors
    /// If the connection can't be established before the timeout, or the handshake fails.
    pub async fn connect(host: &str, port: u16, timeout: Option<Duration>) -> Result<Self, String> {
        let addr = format!("{host}:{port}");
        let timeout = timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT);
        let deadline = tokio::time::Instant::now() + timeout;

        let stream = loop {
            match TcpStream::connect(&addr).await {
                Ok(s) => break s,
                Err(e) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!(
                            "connect to {addr} (is the app running with EGUI_INSPECTION set?): {e}"
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        };
        let _ignored = stream.set_nodelay(true);
        let (mut reader, writer) = stream.into_split();

        // The peer writes the handshake (magic + version) first thing on every connection;
        // read it off the raw stream before the framed transport takes over.
        let mut handshake = [0u8; 8];
        reader
            .read_exact(&mut handshake)
            .await
            .map_err(|e| format!("read handshake: {e}"))?;
        let protocol_version =
            decode_handshake(handshake).map_err(|e| format!("inspection handshake: {e}"))?;

        let mut bridge = Self::with_transport(
            FramedTransport::new(reader, writer),
            PeerInfo {
                transport: addr,
                protocol_version,
                label: None,
            },
        );
        bridge.peer_info.label = bridge.fetch_label().await?;
        Ok(bridge)
    }

    /// Build a bridge over any [`Transport`] implementation (for hosts that own their channel
    /// to the app, e.g. `re_mcp` over gRPC). The caller fills in [`PeerInfo`] since there's no
    /// generic handshake to read.
    pub fn with_transport<T: Transport + 'static>(transport: T, peer_info: PeerInfo) -> Self {
        Self {
            transport: Box::new(transport),
            peer_info,
        }
    }

    /// Send one request and await its single response.
    ///
    /// # Errors
    /// On transport failure sending the request or reading the response.
    pub async fn request(&self, req: Request) -> Result<Response, String> {
        self.transport.request(req).await
    }

    /// The transport this bridge drives the app through.
    pub fn transport(&self) -> &dyn Transport {
        self.transport.as_ref()
    }

    async fn fetch_label(&self) -> Result<Option<String>, String> {
        match self.request(Request::GetInfo).await? {
            Response::Info { label, .. } => Ok(label),
            Response::Error { message } => Err(message),
            _ => Err("unexpected response to GetInfo".to_owned()),
        }
    }

    /// Fetch the current `AccessKit` tree, built into a queryable `accesskit_consumer::Tree`.
    ///
    /// # Errors
    /// On I/O failure or an unexpected response.
    pub async fn fetch_tree(&self) -> Result<TreeSnapshot, String> {
        egui_inspection_tools::transport::fetch_tree(self.transport()).await
    }

    /// Inject input events; resolves after the events have been processed by a frame.
    ///
    /// # Errors
    /// On I/O failure or an unexpected response.
    pub async fn apply_events(&self, events: Vec<egui::Event>) -> Result<(), String> {
        egui_inspection_tools::transport::apply_events(self.transport(), events).await
    }

    /// Resize the viewport (logical points).
    ///
    /// # Errors
    /// On I/O failure or an unexpected response.
    pub async fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        egui_inspection_tools::transport::resize(self.transport(), width, height).await
    }

    /// Capture a screenshot, downscaled to `pixels_per_point` pixels per logical point
    /// (`1.0` = logical size).
    ///
    /// # Errors
    /// On I/O failure or an unexpected response.
    pub async fn screenshot(&self, pixels_per_point: f32) -> Result<EncodedPng, String> {
        egui_inspection_tools::transport::screenshot(self.transport(), pixels_per_point).await
    }
}

// The wire format (length-prefix + MessagePack, capped at `MAX_MESSAGE_BYTES`) is defined
// once in `egui_inspection::protocol`; these are just the async read/write around it.
async fn write_framed<W, T>(writer: &mut W, value: &T) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    writer.write_all(&encode_frame(value)?).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_framed<R, T>(reader: &mut R) -> std::io::Result<T>
where
    R: AsyncRead + Unpin,
    T: for<'de> serde::Deserialize<'de>,
{
    let mut header = [0u8; 4];
    reader.read_exact(&mut header).await?;
    let mut body = vec![0u8; decode_frame_len(header)?];
    reader.read_exact(&mut body).await?;
    decode_frame_body(&body)
}
