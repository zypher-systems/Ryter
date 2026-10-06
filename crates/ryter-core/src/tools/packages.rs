//! `check_package`: a dependency's latest release and its known advisories,
//! from its registry (crates.io, npm, PyPI, the Go module proxy) and from
//! OSV.dev. Fixed public hosts, read-only, no key: the question a builder
//! asks most ("is this current, and is it safe?") answered in one call,
//! without a web search. The plan and audit hats may ask it; the gate
//! decides ([`crate::tools::policy`]).

use serde_json::{Value, json};
use std::time::Duration;

use crate::error::{Error, Result};
use crate::tools::{ToolContext, ToolOutput};

const TIMEOUT: Duration = Duration::from_secs(15);
/// A registry document is read whole; PyPI's can run to megabytes.
const MAX_BYTES: usize = 8_000_000;
/// The most advisories listed; the count says how many there are.
const MAX_ADVISORIES: usize = 12;
/// The most of a free-text field (a summary, a deprecation note) the
/// model is shown.
const MAX_TEXT: usize = 200;

/// `text` cut to [`MAX_TEXT`] characters, on one line.
fn clip(text: &str) -> String {
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > MAX_TEXT {
        let cut: String = one.chars().take(MAX_TEXT).collect();
        format!("{}…", cut.trim_end())
    } else {
        one
    }
}

/// Whether `v` can be a version: printable, no whitespace, short. It is
/// echoed into the report the model trusts, so nothing else gets in.
pub(crate) fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '-' | '+' | '_' | '~' | '^' | '=' | '<' | '>' | '*')
        })
}
const OSV: &str = "https://api.osv.dev/v1/query";

/// The registries the tool knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ecosystem {
    Crates,
    Npm,
    PyPi,
    Go,
}

impl Ecosystem {
    /// The word the model gives, in the spellings it is likely to use.
    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "crates" | "crates.io" | "cargo" | "rust" => Some(Self::Crates),
            "npm" | "node" | "nodejs" | "javascript" | "typescript" => Some(Self::Npm),
            "pypi" | "pip" | "python" => Some(Self::PyPi),
            "go" | "golang" => Some(Self::Go),
            _ => None,
        }
    }

    /// How OSV.dev names it.
    fn osv(self) -> &'static str {
        match self {
            Self::Crates => "crates.io",
            Self::Npm => "npm",
            Self::PyPi => "PyPI",
            Self::Go => "Go",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Crates => "crates.io",
            Self::Npm => "npm",
            Self::PyPi => "PyPI",
            Self::Go => "Go",
        }
    }

    /// Whether `name` is a package name this registry could hold. The name
    /// goes into a URL on a fixed host; nothing but a name is let through.
    pub(crate) fn valid_name(self, name: &str) -> bool {
        if name.is_empty() || name.len() > 214 || name.contains("..") || name.starts_with('/') {
            return false;
        }
        let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
        match self {
            Self::Crates => name.chars().all(plain),
            Self::PyPi => name.chars().all(plain),
            // `@scope/name`: one `/`, after a scope that starts with `@`.
            Self::Npm => {
                let (scope, rest) = match name.strip_prefix('@') {
                    Some(s) => match s.split_once('/') {
                        Some((scope, rest)) => (Some(scope), rest),
                        None => return false,
                    },
                    None => (None, name),
                };
                !rest.is_empty()
                    && rest.chars().all(plain)
                    && scope.is_none_or(|s| !s.is_empty() && s.chars().all(plain))
            }
            // A module path: `github.com/user/repo/v2`.
            Self::Go => name.chars().all(|c| plain(c) || c == '/') && !name.ends_with('/'),
        }
    }

    /// Where the latest release is read from.
    pub(crate) fn registry_url(self, name: &str) -> String {
        match self {
            Self::Crates => format!("https://crates.io/api/v1/crates/{name}"),
            // The `latest` document alone: the whole one runs to megabytes
            // for a popular package.
            Self::Npm => format!(
                "https://registry.npmjs.org/{}/latest",
                name.replace('/', "%2F")
            ),
            Self::PyPi => format!("https://pypi.org/pypi/{name}/json"),
            // The proxy wants an upper-case letter as `!` and the letter
            // in lower case.
            Self::Go => {
                let mut escaped = String::with_capacity(name.len());
                for c in name.chars() {
                    if c.is_ascii_uppercase() {
                        escaped.push('!');
                        escaped.push(c.to_ascii_lowercase());
                    } else {
                        escaped.push(c);
                    }
                }
                format!("https://proxy.golang.org/{escaped}/@latest")
            }
        }
    }
}

/// What a registry says of a package's newest release.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Latest {
    pub version: String,
    /// The day it was published, `YYYY-MM-DD`, where the registry says.
    pub released: Option<String>,
    /// `yanked` or `deprecated: …`, where the registry says.
    pub note: Option<String>,
}

/// One OSV.dev record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Advisory {
    pub id: String,
    pub aliases: Vec<String>,
    pub severity: Option<String>,
    pub summary: String,
    /// The versions that fix it, one per affected range that has one.
    pub fixed: Vec<String>,
}

pub fn check_package(args: &Value, _ctx: &ToolContext) -> Result<ToolOutput> {
    let eco_word = args
        .get("ecosystem")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Config("check_package: missing ecosystem".into()))?;
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .ok_or_else(|| Error::Config("check_package: missing name".into()))?;
    let version = args
        .get("version")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let Some(eco) = Ecosystem::parse(eco_word) else {
        return Ok(ToolOutput::err(format!(
            "check_package: unknown ecosystem {eco_word:?}; one of crates, npm, pypi, go"
        )));
    };
    if !eco.valid_name(name) {
        return Ok(ToolOutput::err(format!(
            "check_package: {name:?} is not a package name {} would hold",
            eco.label()
        )));
    }
    if let Some(v) = version {
        if !valid_version(v) {
            return Ok(ToolOutput::err(format!(
                "check_package: {v:?} is not a version; give it as the registry prints it"
            )));
        }
    }
    let latest = match get_json(&eco.registry_url(name)) {
        Ok(v) => parse_latest(eco, &v),
        Err(e) => Err(e),
    };
    let advisories = post_json(OSV, &osv_query(eco, name, version)).map(|v| parse_osv(&v));
    Ok(ToolOutput::ok(render(
        eco,
        name,
        version,
        &latest,
        &advisories,
    )))
}

fn osv_query(eco: Ecosystem, name: &str, version: Option<&str>) -> Value {
    let mut q = json!({"package": {"name": name, "ecosystem": eco.osv()}});
    if let Some(v) = version {
        q["version"] = Value::String(v.to_string());
    }
    q
}

/// The newest release in the registry's document.
pub(crate) fn parse_latest(eco: Ecosystem, v: &Value) -> Result<Latest> {
    let s = |v: &Value| v.as_str().map(str::to_string);
    let day = |t: Option<String>| t.map(|t| t.chars().take(10).collect::<String>());
    match eco {
        Ecosystem::Crates => {
            let c = &v["crate"];
            let version = s(&c["max_stable_version"])
                .or_else(|| s(&c["max_version"]))
                .ok_or_else(|| Error::Config("crates.io: no version in the document".into()))?;
            let entry = v["versions"]
                .as_array()
                .and_then(|vs| vs.iter().find(|e| e["num"].as_str() == Some(&version)));
            Ok(Latest {
                released: day(entry.and_then(|e| s(&e["created_at"]))),
                note: entry
                    .and_then(|e| e["yanked"].as_bool())
                    .filter(|y| *y)
                    .map(|_| "yanked".to_string()),
                version,
            })
        }
        Ecosystem::Npm => Ok(Latest {
            version: s(&v["version"])
                .ok_or_else(|| Error::Config("npm: no version in the document".into()))?,
            released: None,
            note: s(&v["deprecated"]).map(|d| format!("deprecated: {}", clip(&d))),
        }),
        Ecosystem::PyPi => {
            let version = s(&v["info"]["version"])
                .ok_or_else(|| Error::Config("PyPI: no version in the document".into()))?;
            let files = v["releases"][&version].as_array();
            Ok(Latest {
                released: day(files
                    .and_then(|fs| fs.first())
                    .and_then(|f| s(&f["upload_time"]))),
                note: files
                    .is_some_and(|fs| !fs.is_empty() && fs.iter().all(|f| f["yanked"] == true))
                    .then(|| "yanked".to_string()),
                version,
            })
        }
        Ecosystem::Go => Ok(Latest {
            version: s(&v["Version"])
                .ok_or_else(|| Error::Config("Go proxy: no version in the document".into()))?,
            released: day(s(&v["Time"])),
            note: None,
        }),
    }
}

/// The records in an OSV.dev query answer. A record that is an alias of
/// one already listed (a GHSA and its GO or PYSEC twin) is folded into it:
/// its fixing versions join the first's, so nothing a twin alone knew is
/// lost.
pub(crate) fn parse_osv(v: &Value) -> Vec<Advisory> {
    let Some(vulns) = v["vulns"].as_array() else {
        return Vec::new();
    };
    let mut out: Vec<Advisory> = Vec::new();
    for r in vulns {
        let a = parse_advisory(r);
        match out
            .iter_mut()
            .find(|o| o.aliases.contains(&a.id) || a.aliases.contains(&o.id))
        {
            Some(twin) => {
                for f in a.fixed {
                    if !twin.fixed.contains(&f) {
                        twin.fixed.push(f);
                    }
                }
                if twin.severity.is_none() {
                    twin.severity = a.severity;
                }
            }
            None => out.push(a),
        }
    }
    out
}

/// A commit hash where a version was expected: OSV carries both.
fn is_commit_hash(s: &str) -> bool {
    s.len() >= 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_advisory(r: &Value) -> Advisory {
    let aliases = r["aliases"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    // `database_specific.severity` is a word (`HIGH`); the `severity`
    // list carries a CVSS vector, not one to read, and is left out.
    let severity = r["database_specific"]["severity"]
        .as_str()
        .map(|s| s.to_ascii_lowercase());
    // The one-line summary; failing that, the first sentence of the
    // details. Either cut short.
    let summary = match r["summary"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => clip(s),
        None => {
            let d = r["details"].as_str().unwrap_or("").trim();
            let first = d.lines().next().unwrap_or("");
            clip(
                first
                    .split_inclusive(". ")
                    .next()
                    .unwrap_or(first)
                    .trim_end(),
            )
        }
    };
    let mut fixed: Vec<String> = Vec::new();
    for a in r["affected"].as_array().into_iter().flatten() {
        for range in a["ranges"].as_array().into_iter().flatten() {
            for ev in range["events"].as_array().into_iter().flatten() {
                if let Some(f) = ev["fixed"].as_str() {
                    if !is_commit_hash(f) && !fixed.iter().any(|x| x == f) {
                        fixed.push(f.to_string());
                    }
                }
            }
        }
    }
    Advisory {
        id: r["id"].as_str().unwrap_or("?").to_string(),
        aliases,
        severity,
        summary,
        fixed,
    }
}

/// What the model reads.
pub(crate) fn render(
    eco: Ecosystem,
    name: &str,
    version: Option<&str>,
    latest: &Result<Latest>,
    advisories: &Result<Vec<Advisory>>,
) -> String {
    let mut out = format!("{name} ({})", eco.label());
    match latest {
        Ok(l) => {
            out.push_str(&format!(": latest {}", l.version));
            if let Some(d) = &l.released {
                out.push_str(&format!(", released {d}"));
            }
            if let Some(n) = &l.note {
                out.push_str(&format!(" ({n})"));
            }
            if let Some(v) = version {
                // A string compare: whether it is the latest, not which
                // is newer.
                if v == l.version {
                    out.push_str(&format!("\n{v} is the latest."));
                } else {
                    out.push_str(&format!("\n{v} is in use; the latest is {}.", l.version));
                }
            }
        }
        Err(e) => out.push_str(&format!(": the registry could not be read ({e})")),
    }
    out.push('\n');
    match advisories {
        Ok(list) if list.is_empty() => match version {
            Some(v) => out.push_str(&format!("No known advisories for {v} on OSV.dev.")),
            None => out.push_str("No known advisories for any version on OSV.dev."),
        },
        Ok(list) => {
            let scope = version.map_or("any version".to_string(), str::to_string);
            out.push_str(&format!(
                "{} known advisor{} for {scope} on OSV.dev:",
                list.len(),
                if list.len() == 1 { "y" } else { "ies" }
            ));
            for a in list.iter().take(MAX_ADVISORIES) {
                out.push_str("\n- ");
                out.push_str(&a.id);
                if !a.aliases.is_empty() {
                    out.push_str(&format!(" ({})", a.aliases.join(", ")));
                }
                if let Some(s) = &a.severity {
                    out.push_str(&format!(" {s}"));
                }
                if !a.summary.is_empty() {
                    out.push_str(&format!(": {}", a.summary));
                }
                if !a.fixed.is_empty() {
                    out.push_str(&format!(" — fixed in {}", a.fixed.join(", ")));
                }
            }
            if list.len() > MAX_ADVISORIES {
                out.push_str(&format!("\n… and {} more", list.len() - MAX_ADVISORIES));
            }
        }
        Err(e) => out.push_str(&format!("Advisories could not be read from OSV.dev ({e}).")),
    }
    out
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent(format!(
            "ryter/{} (https://github.com/zypher-systems/ryter)",
            crate::VERSION
        ))
        .build()
        .map_err(|e| Error::Io(e.to_string()))
}

fn body(resp: reqwest::blocking::Response, what: &str) -> Result<Value> {
    let status = resp.status();
    if status.as_u16() == 404 {
        return Err(Error::Config(format!("{what}: not found")));
    }
    if !status.is_success() {
        return Err(Error::Io(format!("{what}: HTTP {}", status.as_u16())));
    }
    use std::io::Read;
    // Read to the cap and one byte more, not the whole answer first.
    let mut bytes = Vec::new();
    resp.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::Io(e.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::Io(format!("{what}: the answer is too large")));
    }
    serde_json::from_slice(&bytes).map_err(|e| Error::Io(format!("{what}: not JSON ({e})")))
}

fn get_json(url: &str) -> Result<Value> {
    let resp = client()?
        .get(url)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| Error::Io(e.to_string()))?;
    body(resp, "registry")
}

fn post_json(url: &str, q: &Value) -> Result<Value> {
    let resp = client()?
        .post(url)
        .json(q)
        .send()
        .map_err(|e| Error::Io(e.to_string()))?;
    body(resp, "OSV.dev")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ecosystem_is_read_in_the_words_a_model_uses() {
        for (w, e) in [
            ("crates", Ecosystem::Crates),
            ("crates.io", Ecosystem::Crates),
            ("Cargo", Ecosystem::Crates),
            ("npm", Ecosystem::Npm),
            ("node", Ecosystem::Npm),
            ("pypi", Ecosystem::PyPi),
            ("pip", Ecosystem::PyPi),
            ("go", Ecosystem::Go),
            ("golang", Ecosystem::Go),
        ] {
            assert_eq!(Ecosystem::parse(w), Some(e), "{w}");
        }
        assert_eq!(Ecosystem::parse("maven"), None);
    }

    /// The name goes into a URL on a fixed host; only a name gets through.
    #[test]
    fn only_a_package_name_reaches_the_registry() {
        assert!(Ecosystem::Crates.valid_name("serde_json"));
        assert!(Ecosystem::Npm.valid_name("@fastify/cookie"));
        assert!(Ecosystem::Npm.valid_name("express"));
        assert!(Ecosystem::PyPi.valid_name("Django"));
        assert!(Ecosystem::Go.valid_name("github.com/gin-gonic/gin"));
        assert!(Ecosystem::Go.valid_name("golang.org/x/net"));
        for (e, bad) in [
            (Ecosystem::Crates, "../../admin"),
            (Ecosystem::Crates, "serde?x=1"),
            (Ecosystem::Crates, "a/b"),
            (Ecosystem::Npm, "@scope"),
            (Ecosystem::Npm, "a/b"),
            (Ecosystem::Npm, "@/x"),
            (Ecosystem::PyPi, ""),
            (Ecosystem::Go, "/etc/passwd"),
            (Ecosystem::Go, "github.com/x/"),
            (Ecosystem::Go, "a b"),
        ] {
            assert!(!e.valid_name(bad), "{e:?} {bad:?}");
        }
        assert_eq!(
            Ecosystem::Npm.registry_url("@fastify/cookie"),
            "https://registry.npmjs.org/@fastify%2Fcookie/latest"
        );
        assert_eq!(
            Ecosystem::Go.registry_url("github.com/BurntSushi/toml"),
            "https://proxy.golang.org/github.com/!burnt!sushi/toml/@latest"
        );
        assert_eq!(
            Ecosystem::Crates.registry_url("serde"),
            "https://crates.io/api/v1/crates/serde"
        );
        assert_eq!(
            Ecosystem::PyPi.registry_url("fastapi"),
            "https://pypi.org/pypi/fastapi/json"
        );
    }

    #[test]
    fn each_registry_document_gives_the_latest_release() {
        let crates = json!({
            "crate": {"max_stable_version": "1.0.219", "max_version": "1.0.219"},
            "versions": [
                {"num": "1.0.219", "created_at": "2025-03-09T19:12:00.000000+00:00", "yanked": false},
                {"num": "1.0.218", "created_at": "2025-02-20T10:00:00.000000+00:00", "yanked": true}
            ]
        });
        assert_eq!(
            parse_latest(Ecosystem::Crates, &crates).unwrap(),
            Latest {
                version: "1.0.219".into(),
                released: Some("2025-03-09".into()),
                note: None
            }
        );
        let npm = json!({"name": "request", "version": "2.88.2", "deprecated": "request has been deprecated"});
        assert_eq!(
            parse_latest(Ecosystem::Npm, &npm).unwrap(),
            Latest {
                version: "2.88.2".into(),
                released: None,
                note: Some("deprecated: request has been deprecated".into())
            }
        );
        let pypi = json!({
            "info": {"version": "0.115.0"},
            "releases": {"0.115.0": [{"upload_time": "2024-09-17T14:03:31", "yanked": false}]}
        });
        assert_eq!(
            parse_latest(Ecosystem::PyPi, &pypi).unwrap(),
            Latest {
                version: "0.115.0".into(),
                released: Some("2024-09-17".into()),
                note: None
            }
        );
        let go = json!({"Version": "v1.10.0", "Time": "2024-05-07T08:24:43Z"});
        assert_eq!(
            parse_latest(Ecosystem::Go, &go).unwrap(),
            Latest {
                version: "v1.10.0".into(),
                released: Some("2024-05-07".into()),
                note: None
            }
        );
        assert!(
            parse_latest(
                Ecosystem::Crates,
                &json!({"errors": [{"detail": "Not Found"}]})
            )
            .is_err()
        );
    }

    #[test]
    fn osv_records_are_read_with_their_fixes() {
        let v = json!({"vulns": [
            {
                "id": "GHSA-xxxx-yyyy-zzzz",
                "aliases": ["CVE-2024-12345"],
                "summary": "Prototype pollution in merge",
                "database_specific": {"severity": "HIGH"},
                "affected": [{"ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "4.17.21"}]}]}]
            },
            {
                "id": "RUSTSEC-2024-0001",
                "details": "Use after free in frobnicate.\n\nMore text.",
                "severity": [{"type": "CVSS_V3", "score": "CVSS:3.1/AV:N/AC:L"}],
                "affected": [{"ranges": [{"type": "SEMVER", "events": [{"introduced": "0.1.0"}]}]}]
            }
        ]});
        let list = parse_osv(&v);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "GHSA-xxxx-yyyy-zzzz");
        assert_eq!(list[0].aliases, vec!["CVE-2024-12345"]);
        assert_eq!(list[0].severity.as_deref(), Some("high"));
        assert_eq!(list[0].fixed, vec!["4.17.21"]);
        assert_eq!(list[1].summary, "Use after free in frobnicate.");
        assert_eq!(list[1].severity, None, "a CVSS vector is not a word");
        assert!(list[1].fixed.is_empty());
        assert!(parse_osv(&json!({})).is_empty());
        // A twin by alias is listed once; a commit hash is not a version;
        // a paragraph of details is cut to its first sentence.
        let v = json!({"vulns": [
            {"id": "GHSA-aaaa", "aliases": ["CVE-2023-1", "GO-2023-1"], "summary": "Bad filename"},
            {"id": "GO-2023-1", "aliases": ["CVE-2023-1", "GHSA-aaaa"], "summary": "Bad filename, again"},
            {"id": "PYSEC-2024-38", "details": "FastAPI is a web framework. When using form data, a RegEx stalls. It was patched in 0.109.1.",
             "affected": [{"ranges": [{"type": "GIT", "events": [{"introduced": "0"}, {"fixed": "9d34ad0ee8a0dfbbcce06f76c2d5d851085024fc"}]},
                                      {"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"fixed": "0.109.1"}]}]}]}
        ]});
        let list = parse_osv(&v);
        assert_eq!(
            list.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
            vec!["GHSA-aaaa", "PYSEC-2024-38"]
        );
        assert_eq!(list[1].summary, "FastAPI is a web framework.");
        assert_eq!(list[1].fixed, vec!["0.109.1"]);
        // A twin's fixing versions join the record kept.
        let v = json!({"vulns": [
            {"id": "GHSA-bbbb", "aliases": ["GO-2024-9"], "summary": "x",
             "affected": [{"ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "1.9.1"}]}]}]},
            {"id": "GO-2024-9", "aliases": ["GHSA-bbbb"], "summary": "x", "database_specific": {"severity": "HIGH"},
             "affected": [{"ranges": [{"type": "SEMVER", "events": [{"introduced": "0"}, {"fixed": "1.9.1"}]},
                                      {"type": "SEMVER", "events": [{"introduced": "2.0.0"}, {"fixed": "2.0.3"}]}]}]}
        ]});
        let list = parse_osv(&v);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].fixed, vec!["1.9.1", "2.0.3"]);
        assert_eq!(list[0].severity.as_deref(), Some("high"));
        // Free text is cut, and a version is checked before it is echoed.
        let cut = clip(&"word ".repeat(100));
        assert!(
            cut.ends_with('…') && cut.chars().count() <= MAX_TEXT + 1,
            "{cut}"
        );
        assert!(valid_version("4.17.15") && valid_version("v1.9.0") && valid_version("^2.0.0"));
        assert!(valid_version("1.0.0-beta.1+build"));
        assert!(!valid_version("4.17.15\nNo known advisories") && !valid_version("1 2"));
        assert!(!valid_version(""));
    }

    #[test]
    fn the_answer_reads_as_one_report() {
        let latest = Ok(Latest {
            version: "4.17.21".into(),
            released: Some("2021-02-20".into()),
            note: None,
        });
        let advisories = Ok(vec![Advisory {
            id: "GHSA-p6mc-m468-83gw".into(),
            aliases: vec!["CVE-2020-28500".into()],
            severity: Some("moderate".into()),
            summary: "ReDoS in toNumber".into(),
            fixed: vec!["4.17.21".into()],
        }]);
        let text = render(
            Ecosystem::Npm,
            "lodash",
            Some("4.17.15"),
            &latest,
            &advisories,
        );
        assert_eq!(
            text,
            "lodash (npm): latest 4.17.21, released 2021-02-20\n4.17.15 is in use; the latest is 4.17.21.\n\
             1 known advisory for 4.17.15 on OSV.dev:\n\
             - GHSA-p6mc-m468-83gw (CVE-2020-28500) moderate: ReDoS in toNumber — fixed in 4.17.21"
        );
        let text = render(
            Ecosystem::Crates,
            "serde",
            Some("1.0.219"),
            &latest_of("1.0.219"),
            &Ok(vec![]),
        );
        assert!(text.contains("1.0.219 is the latest."), "{text}");
        assert!(
            text.ends_with("No known advisories for 1.0.219 on OSV.dev."),
            "{text}"
        );
        let text = render(
            Ecosystem::Go,
            "github.com/x/y",
            None,
            &Err(Error::Config("registry: not found".into())),
            &Err(Error::Io("OSV.dev: HTTP 503".into())),
        );
        assert!(
            text.contains("could not be read (registry: not found)"),
            "{text}"
        );
        assert!(
            text.contains("Advisories could not be read from OSV.dev"),
            "{text}"
        );
    }

    fn latest_of(v: &str) -> Result<Latest> {
        Ok(Latest {
            version: v.into(),
            ..Default::default()
        })
    }

    #[test]
    fn a_bad_ecosystem_or_name_is_refused_before_any_request() {
        let dir = tempfile::TempDir::new().unwrap();
        let ctx = crate::tools::tests::ctx(crate::Role::SoloPlan, dir.path());
        let out = check_package(&json!({"ecosystem": "maven", "name": "x"}), &ctx).unwrap();
        assert!(
            out.is_error && out.text.contains("unknown ecosystem"),
            "{out:?}"
        );
        let out = check_package(&json!({"ecosystem": "crates", "name": "../x"}), &ctx).unwrap();
        assert!(
            out.is_error && out.text.contains("not a package name"),
            "{out:?}"
        );
        let out = check_package(
            &json!({"ecosystem": "crates", "name": "serde", "version": "1.0\nNo known advisories"}),
            &ctx,
        )
        .unwrap();
        assert!(
            out.is_error && out.text.contains("not a version"),
            "{out:?}"
        );
    }
}
