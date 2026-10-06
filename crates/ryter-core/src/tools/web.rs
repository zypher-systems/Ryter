//! `web_fetch` and `web_search` (`[features] web`), for the plan and audit
//! hats. The search goes through a provider of the user's choosing
//! (`[search]`): Tavily with a key, or a SearXNG server of their own.

use regex::Regex;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::time::Duration;

use crate::config::SearchConfig;
use crate::error::{Error, Result};
use crate::tools::{ToolContext, ToolOutput};

const MAX_BYTES: usize = 1_000_000;
const MAX_CHARS: usize = 24_000;
const TIMEOUT: Duration = Duration::from_secs(15);
/// The most of a result's text the model is shown.
const MAX_SNIPPET: usize = 400;
const DEFAULT_RESULTS: u8 = 5;
const MAX_RESULTS: u8 = 10;
const TAVILY: &str = "https://api.tavily.com/search";

/// Where `web_search` looks, with its key read once when the session is
/// made.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Search {
    pub provider: Provider,
    pub max_results: u8,
}

/// A search provider.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum Provider {
    /// `[search]` names none: `web_search` says how to set one.
    #[default]
    None,
    /// Tavily, with the key stored as `tavily` (or `TAVILY_API_KEY`).
    Tavily { key: Option<String> },
    /// A SearXNG server of the user's, by its address.
    Searxng { url: String },
    /// A `provider` word the gate does not know.
    Unknown(String),
}

/// The key never prints: a `{:?}` of the context once would have.
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "None"),
            Self::Tavily { key } => write!(
                f,
                "Tavily {{ key: {} }}",
                if key.is_some() { "Some(…)" } else { "None" }
            ),
            Self::Searxng { url } => write!(f, "Searxng {{ url: {url:?} }}"),
            Self::Unknown(p) => write!(f, "Unknown({p:?})"),
        }
    }
}

/// The variable a Tavily key may be read from. Hidden from every child in
/// [`crate::config::load_at`], before any MCP server starts; the call in
/// [`Search::from_config`] is for a `Search` built without a config load.
pub(crate) const TAVILY_KEY_VAR: &str = "TAVILY_API_KEY";

impl Search {
    /// From `[search]`, reading the key where there is one. The variable
    /// is hidden from every shell and MCP child, set or not, as a
    /// connection's `env_key` is.
    pub fn from_config(cfg: &SearchConfig) -> Self {
        crate::tools::shell::hide_env(TAVILY_KEY_VAR);
        let key = std::env::var(TAVILY_KEY_VAR)
            .ok()
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| crate::config::stored_secret("tavily"));
        Self::from_parts(cfg, key)
    }

    /// [`Self::from_config`] with the key already in hand.
    pub fn from_parts(cfg: &SearchConfig, tavily_key: Option<String>) -> Self {
        let provider = match cfg.provider.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "off" => Provider::None,
            "tavily" => Provider::Tavily { key: tavily_key },
            "searxng" | "searx" => Provider::Searxng {
                url: cfg
                    .url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:8080".to_string())
                    .trim_end_matches('/')
                    .to_string(),
            },
            other => Provider::Unknown(other.to_string()),
        };
        Self {
            provider,
            max_results: match cfg.max_results {
                0 => DEFAULT_RESULTS,
                n => n.min(MAX_RESULTS),
            },
        }
    }
}

/// One result, whichever provider found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub date: Option<String>,
}

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
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| Error::Config("web_search: missing query".into()))?;
    let n = args
        .get("max_results")
        .and_then(Value::as_u64)
        .map_or(ctx.search.max_results, |n| {
            n.clamp(1, u64::from(MAX_RESULTS)) as u8
        });
    let hits = match &ctx.search.provider {
        Provider::None => {
            return Ok(ToolOutput::err(
                "web_search: no search provider is set. Tell the user: in ~/.ryter/config.toml, \
                 `[search] provider = \"tavily\"` with a key (`ryter connections set-key tavily`, \
                 or /provider set-key tavily), or `provider = \"searxng\"` with `url = \
                 \"http://localhost:8080\"` for a server of their own.",
            ));
        }
        Provider::Unknown(p) => {
            return Ok(ToolOutput::err(format!(
                "web_search: unknown provider {p:?} in [search]; tavily or searxng"
            )));
        }
        Provider::Tavily { key: None } => {
            return Ok(ToolOutput::err(
                "web_search: Tavily is set but has no key. Tell the user: `ryter connections \
                 set-key tavily`, /provider set-key tavily, or TAVILY_API_KEY in the environment.",
            ));
        }
        Provider::Tavily { key: Some(key) } => tavily(key, q, n),
        Provider::Searxng { url } => searxng(url, q, n),
    };
    match hits {
        Ok(hits) if hits.is_empty() => Ok(ToolOutput::ok(format!("no results for {q:?}"))),
        Ok(hits) => Ok(ToolOutput::ok(render(&hits))),
        Err(e) => Ok(ToolOutput::err(format!("web_search: {e}"))),
    }
}

fn client(redirects: reqwest::redirect::Policy) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .redirect(redirects)
        .user_agent(format!("ryter/{}", crate::VERSION))
        .build()
        .map_err(|e| Error::Io(e.to_string()))
}

/// Whether a SearXNG redirect may be followed: back to the server the user
/// named, or to any public host; never to a private or metadata address
/// the user did not name. The server itself may be private by the user's
/// configuration; a redirect from it is not theirs.
fn searxng_redirect_allowed(base_host: Option<&str>, next_host: &str) -> bool {
    base_host.is_some_and(|b| b.eq_ignore_ascii_case(next_host)) || !blocked_host(next_host)
}

/// A client for the user's SearXNG: at most three redirects, each judged
/// by [`searxng_redirect_allowed`].
fn searxng_client(base_host: Option<String>) -> Result<reqwest::blocking::Client> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 3 {
            return attempt.error("too many redirects");
        }
        let host = attempt.url().host_str().unwrap_or("").to_string();
        if searxng_redirect_allowed(base_host.as_deref(), &host) {
            attempt.follow()
        } else {
            attempt.error(format!("redirected to a private address ({host})"))
        }
    });
    client(policy)
}

fn json_body(resp: reqwest::blocking::Response, who: &str) -> Result<Value> {
    use std::io::Read;
    let status = resp.status();
    // Read to the cap and one byte more, not the whole answer first.
    let mut raw = Vec::new();
    resp.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|e| Error::Io(e.to_string()))?;
    if raw.len() > MAX_BYTES {
        return Err(Error::Io(format!("{who}: the answer is too large")));
    }
    if !status.is_success() {
        let text = String::from_utf8_lossy(&raw);
        let line = text
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect::<String>();
        return Err(Error::Io(format!("{who}: HTTP {} {line}", status.as_u16())));
    }
    serde_json::from_slice(&raw).map_err(|e| Error::Io(format!("{who}: not JSON ({e})")))
}

fn tavily(key: &str, q: &str, n: u8) -> Result<Vec<Hit>> {
    let resp = client(reqwest::redirect::Policy::limited(3))?
        .post(TAVILY)
        .bearer_auth(key)
        .json(&json!({
            "query": q,
            "max_results": n,
            "search_depth": "basic",
            "include_answer": false,
            "include_raw_content": false,
        }))
        .send()
        .map_err(|e| Error::Io(e.to_string()))?;
    Ok(parse_tavily(&json_body(resp, "Tavily")?))
}

/// A SearXNG server of the user's: its address is theirs to choose, a
/// private one included.
fn searxng(base: &str, q: &str, n: u8) -> Result<Vec<Hit>> {
    let url = format!("{base}/search?q={}&format=json", urlencoding(q));
    let base_host = reqwest::Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    let resp = searxng_client(base_host.clone())?
        .get(&url)
        .send()
        .map_err(|e| Error::Io(e.to_string()))?;
    // The policy judged each hop; the last answer's host once more, as
    // `web_fetch` does.
    if let Some(h) = resp.url().host_str() {
        if !searxng_redirect_allowed(base_host.as_deref(), h) {
            return Err(Error::Config(format!(
                "SearXNG: redirected to a private address ({h})"
            )));
        }
    }
    let mut hits = parse_searxng(&json_body(resp, "SearXNG")?);
    hits.truncate(usize::from(n));
    Ok(hits)
}

fn hit(title: &Value, url: &Value, content: &Value, date: &Value) -> Option<Hit> {
    let url = url.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    let snippet: String = content
        .as_str()
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let snippet = if snippet.chars().count() > MAX_SNIPPET {
        let cut: String = snippet.chars().take(MAX_SNIPPET).collect();
        format!("{}…", cut.trim_end())
    } else {
        snippet
    };
    Some(Hit {
        title: title.as_str().unwrap_or("").trim().to_string(),
        url: url.to_string(),
        snippet,
        date: date
            .as_str()
            .map(|d| d.chars().take(10).collect())
            .filter(|d: &String| !d.is_empty()),
    })
}

/// Tavily's answer: `results[]` of `title`, `url`, `content`,
/// `published_date`.
pub(crate) fn parse_tavily(v: &Value) -> Vec<Hit> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| hit(&r["title"], &r["url"], &r["content"], &r["published_date"]))
        .collect()
}

/// SearXNG's JSON: `results[]` of `title`, `url`, `content`,
/// `publishedDate`.
pub(crate) fn parse_searxng(v: &Value) -> Vec<Hit> {
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| hit(&r["title"], &r["url"], &r["content"], &r["publishedDate"]))
        .collect()
}

/// What the model reads: a numbered list, one result in three lines.
pub(crate) fn render(hits: &[Hit]) -> String {
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!(
            "{}. {}",
            i + 1,
            if h.title.is_empty() { &h.url } else { &h.title }
        ));
        if let Some(d) = &h.date {
            out.push_str(&format!(" ({d})"));
        }
        out.push_str(&format!("\n   {}", h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("\n   {}", h.snippet));
        }
    }
    out
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

    /// `[search]` picks the provider; the key comes with it or is missing,
    /// and the tool says so instead of searching nowhere.
    #[test]
    fn the_search_provider_is_read_from_config() {
        let none = Search::from_parts(&SearchConfig::default(), None);
        assert_eq!(none.provider, Provider::None);
        assert_eq!(none.max_results, DEFAULT_RESULTS);
        let tav = SearchConfig {
            provider: "tavily".into(),
            url: None,
            max_results: 20,
        };
        let s = Search::from_parts(&tav, Some("tvly-x".into()));
        assert_eq!(
            s.provider,
            Provider::Tavily {
                key: Some("tvly-x".into())
            }
        );
        assert_eq!(s.max_results, MAX_RESULTS);
        assert_eq!(
            Search::from_parts(&tav, None).provider,
            Provider::Tavily { key: None }
        );
        let sx = SearchConfig {
            provider: "SearXNG".into(),
            url: Some("http://localhost:8080/".into()),
            max_results: 3,
        };
        assert_eq!(
            Search::from_parts(&sx, None).provider,
            Provider::Searxng {
                url: "http://localhost:8080".into()
            }
        );
        let sx = SearchConfig {
            provider: "searxng".into(),
            url: None,
            max_results: 0,
        };
        assert_eq!(
            Search::from_parts(&sx, None).provider,
            Provider::Searxng {
                url: "http://localhost:8080".into()
            }
        );
        assert_eq!(
            Search::from_parts(
                &SearchConfig {
                    provider: "bing".into(),
                    ..Default::default()
                },
                None
            )
            .provider,
            Provider::Unknown("bing".into())
        );
        // Without a provider or a key, the tool says what to set.
        let dir = tempfile::TempDir::new().unwrap();
        let mut ctx = crate::tools::tests::ctx(crate::Role::SoloPlan, dir.path());
        ctx.web = true;
        let out = web_search(&json!({"query": "fastify cookies"}), &ctx).unwrap();
        assert!(
            out.is_error && out.text.contains("no search provider is set"),
            "{out:?}"
        );
        ctx.search = Search::from_parts(&tav, None);
        let out = web_search(&json!({"query": "fastify cookies"}), &ctx).unwrap();
        assert!(out.is_error && out.text.contains("has no key"), "{out:?}");
    }

    /// The key read from the environment is hidden from every child, and
    /// never printed; a SearXNG redirect goes back to the user's server or
    /// to a public host, never to a private one.
    #[test]
    fn the_key_stays_out_of_children_and_debug_and_redirects_stay_public() {
        let s = Search::from_config(&SearchConfig {
            provider: "tavily".into(),
            ..Default::default()
        });
        assert!(
            crate::tools::shell::hidden_vars()
                .iter()
                .any(|v| v == "TAVILY_API_KEY")
        );
        let _ = s;
        let shown = format!(
            "{:?}",
            Provider::Tavily {
                key: Some("tvly-secret-value".into())
            }
        );
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("Some(…)"), "{shown}");
        assert_eq!(
            format!("{:?}", Provider::Tavily { key: None }),
            "Tavily { key: None }"
        );
        // Literal addresses only: a name would resolve.
        assert!(searxng_redirect_allowed(Some("10.0.0.5"), "10.0.0.5"));
        assert!(searxng_redirect_allowed(Some("10.0.0.5"), "93.184.216.34"));
        assert!(!searxng_redirect_allowed(
            Some("10.0.0.5"),
            "169.254.169.254"
        ));
        assert!(!searxng_redirect_allowed(Some("10.0.0.5"), "127.0.0.1"));
        assert!(!searxng_redirect_allowed(Some("10.0.0.5"), "localhost"));
        assert!(!searxng_redirect_allowed(Some("93.184.216.34"), "10.0.0.5"));
        assert!(!searxng_redirect_allowed(None, "169.254.169.254"));
    }

    /// Both providers' answers read as the same list.
    #[test]
    fn results_from_either_provider_read_the_same() {
        let tav = json!({"query": "x", "results": [
            {"title": "Fastify cookie plugin", "url": "https://github.com/fastify/fastify-cookie", "content": "  A plugin for  Fastify that adds support for reading and setting cookies. ", "score": 0.9, "published_date": "2024-03-01T00:00:00Z"},
            {"title": "", "url": "https://example.com/x", "content": ""},
            {"title": "no url", "content": "dropped"}
        ]});
        let hits = parse_tavily(&tav);
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].snippet,
            "A plugin for Fastify that adds support for reading and setting cookies."
        );
        assert_eq!(hits[0].date.as_deref(), Some("2024-03-01"));
        let sx = json!({"results": [
            {"title": "Fastify cookie plugin", "url": "https://github.com/fastify/fastify-cookie", "content": "A plugin for Fastify that adds support for reading and setting cookies.", "publishedDate": "2024-03-01", "engine": "duckduckgo"}
        ]});
        let same = parse_searxng(&sx);
        assert_eq!(same[0], hits[0]);
        let text = render(&hits);
        assert_eq!(
            text,
            "1. Fastify cookie plugin (2024-03-01)\n   https://github.com/fastify/fastify-cookie\n   A plugin for Fastify that adds support for reading and setting cookies.\n2. https://example.com/x\n   https://example.com/x"
        );
        let long = json!({"results": [{"title": "t", "url": "https://e.com", "content": "word ".repeat(200)}]});
        let h = parse_tavily(&long);
        assert!(h[0].snippet.ends_with('…') && h[0].snippet.chars().count() <= MAX_SNIPPET + 1);
    }

    #[test]
    fn strip_html_drops_tags() {
        let t = strip_html("<html><script>x</script><p>Hello <b>world</b></p>");
        assert!(t.contains("Hello"));
        assert!(t.contains("world"));
        assert!(!t.contains("script"));
    }
}
