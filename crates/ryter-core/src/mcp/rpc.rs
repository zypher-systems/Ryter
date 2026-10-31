//! JSON-RPC 2.0 line protocol (MCP stdio).

use std::io::{BufRead, ErrorKind};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One line read from an MCP peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A whole line, without its line ending.
    Line(String),
    /// A line longer than the reader's ceiling. Nothing after it can be
    /// trusted to start a message, so the connection ends here.
    TooLong,
}

/// The next line of at most `most` bytes, `None` at the end of the stream.
/// `BufRead::lines` holds a line in memory until its newline comes, and a
/// peer need never send one.
pub fn read_frame(reader: &mut impl BufRead, most: usize) -> std::io::Result<Option<Frame>> {
    let mut line = Vec::new();
    loop {
        let got = match reader.fill_buf() {
            Ok(got) => got,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if got.is_empty() {
            break;
        }
        let newline = got.iter().position(|b| *b == b'\n');
        let text = newline.unwrap_or(got.len());
        if line.len() + text > most {
            return Ok(Some(Frame::TooLong));
        }
        line.extend_from_slice(&got[..text]);
        reader.consume(text + usize::from(newline.is_some()));
        if newline.is_some() {
            return finished(line).map(Some);
        }
    }
    if line.is_empty() {
        return Ok(None);
    }
    finished(line).map(Some)
}

fn finished(mut line: Vec<u8>) -> std::io::Result<Frame> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line)
        .map(Frame::Line)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
}

/// Request id.
pub type RpcId = Value;

/// A JSON-RPC request or notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    /// Always `2.0`.
    #[serde(default = "two")]
    pub jsonrpc: String,
    /// Missing on notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RpcId>,
    /// Method name.
    pub method: String,
    /// Params object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

fn two() -> String {
    "2.0".into()
}

/// A JSON-RPC response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    /// Always `2.0`.
    pub jsonrpc: String,
    /// Request id.
    pub id: RpcId,
    /// Result.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    /// Code.
    pub code: i32,
    /// Message.
    pub message: String,
}

impl RpcResponse {
    /// Success.
    pub fn ok(id: RpcId, result: Value) -> Self {
        Self {
            jsonrpc: two(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Failure.
    pub fn err(id: RpcId, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: two(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }

    /// Encode as one JSON line (no trailing spaces).
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        s.push('\n');
        s
    }
}

impl RpcRequest {
    /// Encode as one JSON line.
    pub fn to_line(&self) -> String {
        let mut s = serde_json::to_string(self).unwrap_or_else(|_| "{}".into());
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_request() {
        let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let r: RpcRequest = serde_json::from_str(line).unwrap();
        assert_eq!(r.method, "tools/list");
        assert_eq!(r.id, Some(json!(1)));
    }
}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::io::Cursor;

    fn all(text: &[u8], most: usize) -> Vec<Frame> {
        let mut reader = Cursor::new(text.to_vec());
        let mut frames = Vec::new();
        while let Some(frame) = read_frame(&mut reader, most).unwrap() {
            let over = frame == Frame::TooLong;
            frames.push(frame);
            if over {
                break;
            }
        }
        frames
    }

    #[test]
    fn a_frame_is_a_line_without_its_ending() {
        let line = |s: &str| Frame::Line(s.into());
        assert_eq!(all(b"ab\r\n\ncd", 3), [line("ab"), line(""), line("cd")]);
        assert_eq!(all(b"", 2), []);
    }

    #[test]
    fn a_line_past_the_ceiling_stops_without_waiting_for_its_newline() {
        assert_eq!(all(b"abc", 2), [Frame::TooLong]);
        assert_eq!(
            all(b"ok\nabcd\nnext\n", 2),
            [Frame::Line("ok".into()), Frame::TooLong]
        );
    }

    /// One read that never holds more than the ceiling, however long the
    /// line: the bytes after the ceiling stay in the peer's hands.
    #[test]
    fn a_long_line_is_refused_at_the_ceiling_across_reads() {
        let mut reader = std::io::BufReader::with_capacity(4, Cursor::new(vec![b'x'; 100]));
        assert_eq!(read_frame(&mut reader, 10).unwrap(), Some(Frame::TooLong));
    }

    #[test]
    fn a_line_that_is_not_text_is_an_error() {
        let mut reader = Cursor::new(vec![0xff, b'\n']);
        assert!(read_frame(&mut reader, 10).is_err());
    }
}
