//! Optional web_fetch / web_search (`[features] web = true`).

use regex::Regex;
use serde_json::Value;
use std::net::IpAddr;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::tools::{ToolContext, ToolOutput};

const MAX_BYTES: usize = 1_000_000;
const MAX_CHARS: usize = 24_000;
const TIMEOUT: Duration = Duration::from_secs(15);

pub fn web_fetch(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    if !ctx.web {
        return Ok(ToolOutput::err(
            "web_fetch disabled ([features] web = false)",
        ));
    }
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("web_fetch: missing url".into()))?;
    match get_text(url) {
        Ok(t) => Ok(ToolOutput::ok(t)),
        Err(e) => Ok(ToolOutput::err(e.to_string())),
    }
}

pub fn web_search(args: &Value, ctx: &ToolContext) -> Result<ToolOutput> {
    if !ctx.web {
        return Ok(ToolOutput::err(
            "web_search disabled ([features] web = false)",
        ));
    }
    let q = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("web_search: missing query".into()))?;
    let encoded: String = urlencoding(q);
    let url = format!("https://lite.duckduckgo.com/lite/?q={encoded}");
    let html = match get_text(&url) {
        Ok(t) => t,
        Err(e) => return Ok(ToolOutput::err(e.to_string())),
    };
    let links = extract_results(&html);
    if links.is_empty() {
        return Ok(ToolOutput::ok("no search results parsed"));
    }
    Ok(ToolOutput::ok(links.join("\n")))
}

fn get_text(url: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| Error::Config(format!("url: {e}")))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(Error::Config("only http/https URLs are allowed".into()));
    }
    let host = parsed.host_str().unwrap_or("");
    if blocked_host(host) {
        return Err(Error::Config(format!("blocked host {host}")));
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .user_agent(format!("ryter/{}", crate::VERSION))
        .build()
        .map_err(|e| Error::Io(e.to_string()))?;
    let resp = client
        .get(parsed)
        .send()
        .map_err(|e| Error::Io(e.to_string()))?;
    if let Some(h) = resp.url().host_str() {
        if blocked_host(h) {
            return Err(Error::Config(format!("redirected to blocked host {h}")));
        }
    }
    let status = resp.status();
    let raw = resp.bytes().map_err(|e| Error::Io(e.to_string()))?;
    if raw.len() > MAX_BYTES {
        return Err(Error::Io("response larger than 1MB".into()));
    }
    let text = String::from_utf8_lossy(&raw);
    let stripped = strip_html(&text);
    let mut out = stripped.chars().take(MAX_CHARS).collect::<String>();
    if !status.is_success() {
        out = format!("HTTP {status}\n{out}");
    }
    Ok(out)
}

fn blocked_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_nonpublic(ip);
    }
    false
}

fn is_nonpublic(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_multicast()
                || v.octets()[0] == 169 && v.octets()[1] == 254
                || v.octets()[0] == 0
        }
        IpAddr::V6(v) => {
            v.is_loopback() || v.is_multicast() || v.is_unique_local() || v.is_unspecified()
        }
    }
}

fn strip_html(s: &str) -> String {
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").ok();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").ok();
    let re_tag = Regex::new(r"(?is)<[^>]+>").ok();
    let mut t = s.to_string();
    if let Some(r) = &re_script {
        t = r.replace_all(&t, " ").into_owned();
    }
    if let Some(r) = &re_style {
        t = r.replace_all(&t, " ").into_owned();
    }
    if let Some(r) = &re_tag {
        t = r.replace_all(&t, " ").into_owned();
    }
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_results(html: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r#"(?is)href="(https?://[^"]+)"[^>]*>([^<]{3,120})"#) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for cap in re.captures_iter(html) {
        let url = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let title = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if url.contains("duckduckgo.com") || title.is_empty() {
            continue;
        }
        out.push(format!("{title}\n  {url}"));
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn urlencoding(s: &str) -> String {
    let mut o = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                o.push(*b as char);
            }
            b' ' => o.push('+'),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_localhost_and_metadata() {
        assert!(blocked_host("localhost"));
        assert!(blocked_host("127.0.0.1"));
        assert!(blocked_host("169.254.169.254"));
        assert!(blocked_host("10.0.0.1"));
        assert!(!blocked_host("example.com"));
    }

    #[test]
    fn strip_html_drops_tags() {
        let t = strip_html("<html><script>x</script><p>Hello <b>world</b></p>");
        assert!(t.contains("Hello"));
        assert!(t.contains("world"));
        assert!(!t.contains("script"));
    }
}
