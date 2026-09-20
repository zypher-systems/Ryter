//! stdio MCP round-trip against `ryter mcp echo`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[test]
fn echo_initialize_list_call() {
    let exe = env!("CARGO_BIN_EXE_ryter");
    let mut child = Command::new(exe)
        .args(["mcp", "echo"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn echo");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("ryter"), "{line}");
    assert!(line.contains("protocolVersion"), "{line}");

    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    stdin.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("ryter_prompt"), "{line}");
    assert!(line.contains("echo"), "{line}");

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"echo","arguments":{{"message":"pong"}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("pong"), "{line}");

    drop(stdin);
    let _ = child.wait();
}
