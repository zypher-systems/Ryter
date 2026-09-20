//! Unix-socket and TCP listeners for inbound MCP.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::mcp::server::{InboundHost, serve_session};

/// Default attach socket (`~/.ryter/ryter.sock`).
pub fn default_socket_path(home: &Path) -> PathBuf {
    home.join("ryter.sock")
}

/// Bind a Unix socket, replacing a stale file if nothing is listening.
#[cfg(unix)]
pub fn bind_unix(path: &Path) -> Result<std::os::unix::net::UnixListener> {
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => {
                return Err(Error::Config(format!(
                    "socket {} is already in use",
                    path.display()
                )));
            }
            Err(_) => {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(listener)
}

/// Serve inbound MCP on `path` until the process exits.
#[cfg(unix)]
pub fn serve_unix(path: &Path, host: Arc<dyn InboundHost>) -> Result<()> {
    let listener = bind_unix(path)?;
    eprintln!("ryter serve {}", path.display());
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let host = host.clone();
        std::thread::spawn(move || {
            let Ok(clone) = stream.try_clone() else {
                return;
            };
            let _ = serve_session(clone, stream, host.as_ref(), &[]);
        });
    }
    Ok(())
}

/// TCP bind checks: token required; unspecified address needs `i_mean_it`.
pub fn check_tcp(addr: SocketAddr, token: &str, i_mean_it: bool) -> Result<()> {
    if token.is_empty() {
        return Err(Error::Config(
            "TCP inbound requires --token or RYTER_MCP_TOKEN".into(),
        ));
    }
    if addr.ip().is_unspecified() && !i_mean_it {
        return Err(Error::Config(
            "refusing to bind 0.0.0.0/:: without --i-mean-it".into(),
        ));
    }
    Ok(())
}

/// Serve inbound MCP on TCP. Each connection must `initialize` with a listed token.
pub fn serve_tcp(addr: SocketAddr, host: Arc<dyn InboundHost>, tokens: Vec<String>) -> Result<()> {
    let listener = TcpListener::bind(addr).map_err(|e| Error::Io(e.to_string()))?;
    let bound = listener.local_addr().unwrap_or(addr);
    eprintln!("ryter serve {bound}");
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let host = host.clone();
        let tokens = tokens.clone();
        std::thread::spawn(move || {
            handle_tcp(stream, host, tokens);
        });
    }
    Ok(())
}

fn handle_tcp(stream: TcpStream, host: Arc<dyn InboundHost>, tokens: Vec<String>) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let _ = serve_session(clone, stream, host.as_ref(), &tokens);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::server::EchoHost;
    use crate::mcp::{RpcRequest, RpcResponse};
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use tempfile::TempDir;

    fn rpc_line(method: &str, id: u64, params: serde_json::Value) -> String {
        RpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(id)),
            method: method.into(),
            params: Some(params),
        }
        .to_line()
    }

    #[test]
    fn unix_echo_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ryter.sock");
        let host: Arc<dyn InboundHost> = Arc::new(EchoHost::default());
        let listener = bind_unix(&path).unwrap();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                let stream = stream.0;
                let clone = stream.try_clone().unwrap();
                let _ = serve_session(clone, stream, host.as_ref(), &[]);
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        let mut client = UnixStream::connect(&path).unwrap();
        client
            .write_all(
                rpc_line(
                    "initialize",
                    1,
                    json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
                )
                .as_bytes(),
            )
            .unwrap();
        client.flush().unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let resp: RpcResponse = serde_json::from_str(&line).unwrap();
        assert!(resp.result.is_some(), "{line}");
        client
            .write_all(
                rpc_line(
                    "tools/call",
                    2,
                    json!({"name":"echo","arguments":{"message":"pong"}}),
                )
                .as_bytes(),
            )
            .unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("pong"), "{line}");
    }

    #[test]
    fn tcp_rejects_unspecified_without_flag() {
        let addr: SocketAddr = "0.0.0.0:9".parse().unwrap();
        let err = check_tcp(addr, "secret", false).unwrap_err();
        assert!(err.to_string().contains("i-mean-it"), "{err}");
        assert!(check_tcp(addr, "secret", true).is_ok());
        let loopback: SocketAddr = "127.0.0.1:9".parse().unwrap();
        assert!(check_tcp(loopback, "", false).is_err());
        assert!(check_tcp(loopback, "secret", false).is_ok());
    }

    #[test]
    fn tcp_requires_token_on_initialize() {
        let host: Arc<dyn InboundHost> = Arc::new(EchoHost::default());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok(stream) = listener.accept() {
                handle_tcp(stream.0, host, vec!["s3cret".into()]);
            }
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client
            .write_all(
                rpc_line(
                    "initialize",
                    1,
                    json!({"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}),
                )
                .as_bytes(),
            )
            .unwrap();
        client.flush().unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains("unauthorized"), "{line}");
    }
}
