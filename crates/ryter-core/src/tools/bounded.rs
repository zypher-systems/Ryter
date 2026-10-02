//! Bounded capture for pipes and line-oriented readers.

use std::collections::VecDeque;
use std::io::{self, BufRead};

pub(super) struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    half: usize,
    pub total: usize,
}

impl Capture {
    pub fn new(cap: usize) -> Self {
        Self {
            head: Vec::new(),
            tail: VecDeque::new(),
            half: cap / 2,
            total: 0,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.total = self.total.saturating_add(bytes.len());
        let n = self.half.saturating_sub(self.head.len()).min(bytes.len());
        self.head.extend_from_slice(&bytes[..n]);
        let rest = &bytes[n..];
        if rest.len() >= self.half {
            self.tail.clear();
            self.tail.extend(&rest[rest.len() - self.half..]);
        } else {
            let drop = (self.tail.len() + rest.len()).saturating_sub(self.half);
            self.tail.drain(..drop);
            self.tail.extend(rest);
        }
    }

    pub fn recent(&self, n: usize) -> Vec<u8> {
        self.head
            .iter()
            .chain(self.tail.iter())
            .rev()
            .take(n)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn render(&self) -> String {
        let dropped = self.total.saturating_sub(self.head.len() + self.tail.len());
        let mut bytes = self.head.clone();
        if dropped > 0 {
            bytes.extend_from_slice(
                format!("\n… {dropped} bytes elided while reading output …\n").as_bytes(),
            );
        }
        bytes.extend(self.tail.iter());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

pub(super) struct Line {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

impl Line {
    pub fn text(mut self) -> io::Result<String> {
        if self.truncated {
            // A retained prefix may end in the middle of a valid UTF-8 code point.
            if let Err(e) = std::str::from_utf8(&self.bytes) {
                if e.error_len().is_none() {
                    self.bytes.truncate(e.valid_up_to());
                }
            }
        }
        String::from_utf8(self.bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

/// Read a logical line with bounded storage, draining its unretained suffix.
/// `cap == 0` skips a line without allocating its contents.
pub(super) fn line(
    reader: &mut impl BufRead,
    cap: usize,
    cancel: &crate::Cancel,
) -> io::Result<Option<Line>> {
    let mut bytes = Vec::new();
    let mut seen = 0usize;
    loop {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok((seen > 0).then_some(Line {
                truncated: seen > bytes.len(),
                bytes,
            }));
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let n = newline.unwrap_or(chunk.len());
        let keep = n.min(cap.saturating_sub(bytes.len()));
        bytes.extend_from_slice(&chunk[..keep]);
        seen = seen.saturating_add(n);
        reader.consume(n + usize::from(newline.is_some()));
        if newline.is_some() {
            let truncated = seen > bytes.len();
            if !truncated && bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
            return Ok(Some(Line { bytes, truncated }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};

    #[test]
    fn capture_keeps_both_ends_with_constant_storage() {
        let mut capture = Capture::new(1024);
        capture.push(b"first marker\n");
        for _ in 0..1000 {
            capture.push(&[b'x'; 8192]);
        }
        capture.push(b"\nlast marker");
        assert!(capture.head.len() + capture.tail.len() <= 1024);
        assert_eq!(capture.total, 8_192_025);
        let text = capture.render();
        assert!(text.starts_with("first marker"));
        assert!(text.ends_with("last marker"));
        assert!(text.contains("bytes elided"));
    }

    #[test]
    fn giant_lines_are_drained_and_the_following_line_remains_available() {
        let input = io::repeat(b'x')
            .take(5_000_000)
            .chain(io::Cursor::new(b"\nnext\r\n"));
        let mut reader = BufReader::new(input);
        let cancel = crate::Cancel::new();
        let first = line(&mut reader, 4096, &cancel).unwrap().unwrap();
        assert!(first.truncated);
        assert_eq!(first.bytes.len(), 4096);
        assert_eq!(
            line(&mut reader, 4096, &cancel)
                .unwrap()
                .unwrap()
                .text()
                .unwrap(),
            "next"
        );
        assert!(line(&mut reader, 4096, &cancel).unwrap().is_none());
    }
}
