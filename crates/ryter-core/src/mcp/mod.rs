//! MCP JSON-RPC (stdio): outbound client and inbound server.

mod client;
mod listen;
mod rpc;
mod server;

pub use client::{McpHub, RemoteTool, search_tools, use_tool};
pub use listen::{check_tcp, default_socket_path, serve_tcp};
pub use rpc::{RpcError, RpcId, RpcRequest, RpcResponse};
pub use server::{EchoHost, InboundHost, StatusSnapshot, serve_echo, serve_inbound, serve_session};

#[cfg(unix)]
pub use listen::{bind_unix, serve_unix};
