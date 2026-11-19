use serde_json::Value;

/// One item of a tool's result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// Text for the agent to read. Often JSON.
    Text(String),

    /// An encoded image, e.g. a `screenshot`.
    Image {
        /// The encoded image file, e.g. a PNG.
        bytes: Vec<u8>,

        /// The MIME type of `bytes`, e.g. `image/png`.
        mime_type: String,
    },
}

/// The result of one tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolOutput {
    /// What the agent reads, in order.
    pub content: Vec<Content>,

    /// The result as a JSON value, for tools that return one.
    ///
    /// When set, `content` holds the same value as a single [`Content::Text`].
    pub structured: Option<Value>,

    /// The call failed. `content` says why.
    ///
    /// A failed call is an answer for the agent to read and recover from (e.g. "no widget
    /// matches"), not a fault in the host.
    pub is_error: bool,
}

impl ToolOutput {
    /// A successful result with the given content.
    pub fn success(content: Vec<Content>) -> Self {
        Self {
            content,
            structured: None,
            is_error: false,
        }
    }

    /// A successful result carrying a JSON value, both as text and as [`Self::structured`].
    pub fn structured(value: Value) -> Self {
        Self {
            content: vec![Content::Text(value.to_string())],
            structured: Some(value),
            is_error: false,
        }
    }

    /// A failed call, with a message for the agent.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![Content::Text(message.into())],
            structured: None,
            is_error: true,
        }
    }
}
