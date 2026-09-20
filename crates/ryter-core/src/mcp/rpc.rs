//! JSON-RPC 2.0 line protocol (MCP stdio).

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
