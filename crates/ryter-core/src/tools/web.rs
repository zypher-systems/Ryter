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

/// Hostnames that never resolve anywhere useful, plus the well-known cloud
/// metadata names. Checked before resolution so they fail even if DNS lies.
const BLOCKED_NAMES: &[&str] = &[
    "localhost",
    "metadata",
    "metadata.google.internal",
    "metadata.goog",
    "instance-data",
];

fn blocked_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    let h = h.trim_end_matches('.');
    if BLOCKED_NAMES.contains(&h)
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".arpa")
    {
        return true;
    }
    if let Ok(ip) = h.parse::<IpAddr>() {
        return is_nonpublic(ip);
    }
    // A name is only safe if every address it resolves to is public. Checking
    // the string alone let `metadata.google.internal` and any attacker-owned
    // name pointing at 169.254.169.254 straight through.
    match resolve_host(h) {
        Ok(ips) => ips.is_empty() || ips.iter().any(|ip| is_nonpublic(*ip)),
        // Refuse what cannot be checked.
        Err(_) => true,
    }
}

/// Every address `host` resolves to. The port is irrelevant to the check.
fn resolve_host(host: &str) -> std::io::Result<Vec<IpAddr>> {
    use std::net::ToSocketAddrs;
    Ok((host, 80u16).to_socket_addrs()?.map(|sa| sa.ip()).collect())
}

fn is_nonpublic(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_multicast()
                || v.is_broadcast()
                || v.is_documentation()
                || v.octets()[0] == 0
                // Carrier-grade NAT and benchmarking ranges.
                || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1]))
                || (v.octets()[0] == 198 && (18..20).contains(&v.octets()[1]))
                // Reserved / shared address space.
                || v.octets()[0] >= 240
        }
        IpAddr::V6(v) => {
            // An IPv4-mapped address is an IPv4 address wearing a hat:
            // `::ffff:169.254.169.254` is not loopback, private, or unique
            // local, so it walked past the old check.
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_nonpublic(IpAddr::V4(v4));
            }
            v.is_loopback()
                || v.is_multicast()
                || v.is_unique_local()
                || v.is_unspecified()
                || v.is_unicast_link_local()
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
        // Literals and names, decided without touching the network.
        for host in [
            "localhost",
            "LOCALHOST",
            "foo.localhost",
            "db.local",
            "metadata",
            "metadata.google.internal",
            "metadata.google.internal.",
            "anything.internal",
            "169.254.169.254.in-addr.arpa",
            "127.0.0.1",
            "169.254.169.254",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "0.0.0.0",
            "100.64.0.1",
            "[::1]",
            "[fe80::1]",
            "[fd00::1]",
        ] {
            assert!(blocked_host(host), "{host} should be blocked");
        }
    }

    /// `::ffff:169.254.169.254` is an IPv4 address in an IPv6 suit: none of
    /// `is_loopback` / `is_private` / `is_unique_local` catch it.
    #[test]
    fn ipv4_mapped_ipv6_is_unwrapped() {
        for host in [
            "[::ffff:127.0.0.1]",
            "[::ffff:169.254.169.254]",
            "[::ffff:10.0.0.1]",
        ] {
            assert!(blocked_host(host), "{host} should be blocked");
        }
        assert!(is_nonpublic("::ffff:169.254.169.254".parse().unwrap()));
        assert!(!is_nonpublic("::ffff:93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn public_literals_are_allowed() {
        for host in ["93.184.216.34", "1.1.1.1", "[2606:4700:4700::1111]"] {
            assert!(!blocked_host(host), "{host} should be allowed");
        }
    }

    /// A name that resolves only to public addresses is allowed; one that
    /// cannot be resolved is refused rather than assumed safe.
    #[test]
    fn names_are_judged_by_what_they_resolve_to() {
        assert!(
            blocked_host("this-name-does-not-exist.ryter-test.invalid"),
            "unresolvable names must be refused"
        );
        // Only meaningful with working DNS; skip offline.
        if resolve_host("example.com").is_ok() {
            assert!(!blocked_host("example.com"));
        }
    }

    #[test]
    fn strip_html_drops_tags() {
        let t = strip_html("<html><script>x</script><p>Hello <b>world</b></p>");
        assert!(t.contains("Hello"));
        assert!(t.contains("world"));
        assert!(!t.contains("script"));
    }
}
