//! Unix-socket and TCP listeners for inbound MCP.

use std::io::Write;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::mcp::rpc::RpcResponse;
use crate::mcp::server::{InboundHost, serve_session};

/// Connections a listener serves at once. Each holds threads and its queue.
const MAX_CONNECTIONS: usize = 32;
/// How long a reply may wait on a peer that has stopped reading.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// A connection counted against a listener's cap until it ends.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serve each connection on a thread of its own, `most` at a time. One past
/// that is told so and closed.
fn accept<S>(
    incoming: impl Iterator<Item = std::io::Result<S>>,
    most: usize,
    serve: impl Fn(S) + Send + Clone + 'static,
) where
    S: Write + Send + 'static,
{
    let open = Arc::new(AtomicUsize::new(0));
    for stream in incoming {
        let Ok(mut stream) = stream else {
            continue;
        };
        let taken = open.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < most).then_some(n + 1)
        });
        if taken.is_err() {
            let why = format!("busy; {most} connections are open");
            let line = RpcResponse::err(Value::Null, -32000, why).to_line();
            let _ = stream.write_all(line.as_bytes());
            continue;
        }
        let slot = Slot(open.clone());
        let serve = serve.clone();
        std::thread::spawn(move || {
            let _slot = slot;
            serve(stream);
        });
    }
}

/// Default attach socket (`~/.ryter/ryter.sock`).
pub fn default_socket_path(home: &Path) -> PathBuf {
    home.join("ryter.sock")
}

/// Bind a new Unix socket without removing anything already at the path.
/// A stale socket must be removed explicitly after checking its owner.
#[cfg(unix)]
pub fn bind_unix(path: &Path) -> Result<std::os::unix::net::UnixListener> {
    // macOS bind can follow a dangling symlink. exists() misses those;
    // inspect the entry itself and fail closed on any inspection error.
    match path.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Error::Io(format!(
                "{} already exists; choose another socket path or explicitly remove a confirmed stale socket",
                path.display()
            )));
        }
        Err(e) => return Err(Error::Io(format!("{}: {e}", path.display()))),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::Io(e.to_string()))?;
    }
    // Let bind claim the name atomically. Even an inode check followed by
    // unlink can delete a different file substituted in between. A failed
    // connection is not proof of a stale socket either (permissions, etc.).
    let listener = std::os::unix::net::UnixListener::bind(path).map_err(|e| {
        Error::Io(format!(
            "{}: {e}. Existing paths are preserved; choose another socket path, \
             or remove a stale socket only after checking no server owns it",
            path.display()
        ))
    })?;
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
    accept(listener.incoming(), MAX_CONNECTIONS, move |stream| {
        handle_unix(stream, &host);
    });
    Ok(())
}

#[cfg(unix)]
fn handle_unix(stream: std::os::unix::net::UnixStream, host: &Arc<dyn InboundHost>) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let _ = serve_session(clone, &stream, host.as_ref(), &[]);
    // The reader holds its own handle; only this ends its read.
    let _ = stream.shutdown(Shutdown::Both);
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
    accept(listener.incoming(), MAX_CONNECTIONS, move |stream| {
        handle_tcp(stream, host.clone(), tokens.clone());
    });
    Ok(())
}

fn handle_tcp(stream: TcpStream, host: Arc<dyn InboundHost>, tokens: Vec<String>) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let _ = serve_session(clone, &stream, host.as_ref(), &tokens);
    // The reader holds its own handle: without this the peer is never told,
    // and a refused connection stayed open with a thread parked on it.
    let _ = stream.shutdown(Shutdown::Both);
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
    fn binding_preserves_every_existing_path() {
        use std::os::unix::fs::{FileTypeExt, symlink};
        use std::os::unix::net::UnixListener;
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("file");
        std::fs::write(&file, "user data").unwrap();
        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        let link = dir.path().join("link");
        symlink(&file, &link).unwrap();
        let dangling = dir.path().join("dangling");
        symlink(dir.path().join("absent"), &dangling).unwrap();
        let live = dir.path().join("live.sock");
        let listener = UnixListener::bind(&live).unwrap();
        let stale = dir.path().join("stale.sock");
        drop(UnixListener::bind(&stale).unwrap());
        for path in [&file, &folder, &link, &dangling, &live, &stale] {
            assert!(bind_unix(path).is_err(), "{}", path.display());
            assert!(path.symlink_metadata().is_ok());
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "user data");
        assert_eq!(std::fs::read_link(&link).unwrap(), file);
        assert!(folder.is_dir());
        assert!(stale.symlink_metadata().unwrap().file_type().is_socket());
        let client = UnixStream::connect(&live).unwrap();
        let _accepted = listener.accept().unwrap();
        drop(client);
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

    /// Read to the end of the stream: the error a session ended on, and
    /// that it did end.
    fn last_error(stream: &TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut said = String::new();
        BufReader::new(stream).read_line(&mut said).unwrap();
        let mut rest = String::new();
        assert_eq!(
            BufReader::new(stream).read_line(&mut rest).unwrap(),
            0,
            "the connection stayed open"
        );
        let reply: RpcResponse = serde_json::from_str(&said).unwrap();
        reply.error.unwrap().message
    }

    fn tcp_session(tokens: &[&str]) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let tokens: Vec<String> = tokens.iter().map(|t| t.to_string()).collect();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                handle_tcp(stream, Arc::new(EchoHost::default()), tokens);
            }
        });
        client
    }

    #[test]
    fn a_line_past_the_ceiling_ends_the_session_before_authentication() {
        let mut client = tcp_session(&["secret"]);
        let line = vec![b'x'; crate::mcp::server::MAX_LINE + 1];
        client.write_all(&line).unwrap();
        assert_eq!(last_error(&client), "a line over 4 MiB");
    }

    #[test]
    fn a_peer_that_never_proves_itself_is_closed() {
        let client = tcp_session(&["secret"]);
        assert_eq!(last_error(&client), "no valid initialize in time");
    }

    /// `unauthorized` was sent and the socket left open.
    #[test]
    fn a_refused_token_closes_the_connection() {
        let mut client = tcp_session(&["secret"]);
        client
            .write_all(rpc_line("initialize", 1, json!({"token": "wrong"})).as_bytes())
            .unwrap();
        assert_eq!(last_error(&client), "unauthorized");
    }

    #[test]
    fn a_listener_serves_its_cap_and_turns_away_the_next() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            accept(listener.incoming(), 2, |stream| {
                handle_tcp(stream, Arc::new(EchoHost::default()), Vec::new());
            });
        });
        let ping = |client: &mut TcpStream| {
            client
                .write_all(rpc_line("ping", 1, json!({})).as_bytes())
                .unwrap();
            let mut said = String::new();
            BufReader::new(&*client).read_line(&mut said).unwrap();
            assert!(said.contains("\"result\""), "{said}");
        };
        let mut first = TcpStream::connect(addr).unwrap();
        ping(&mut first);
        let mut second = TcpStream::connect(addr).unwrap();
        ping(&mut second);
        let turned_away = TcpStream::connect(addr).unwrap();
        assert_eq!(last_error(&turned_away), "busy; 2 connections are open");
        // One that leaves gives its place back.
        drop(first);
        let served = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            let client = TcpStream::connect(addr).unwrap();
            client
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut client = client;
            let _ = client.write_all(rpc_line("ping", 1, json!({})).as_bytes());
            let mut said = String::new();
            BufReader::new(&client).read_line(&mut said).is_ok() && said.contains("\"result\"")
        });
        assert!(served, "the slot was never given back");
        drop(second);
    }
}
