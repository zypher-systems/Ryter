//! `ryter bench` as a gate: a run that measured nothing fails, and never
//! replaces the run published before it.
//!
//! Every case here exited 0 once. Nothing calls a real model: the provider
//! is a listener in this process that refuses every key.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

/// A run published earlier: one task, accepted.
const PUBLISHED: &str = r#"{
  "ryter": "0.0.1",
  "date": "2026-01-01",
  "lead": "lead-model",
  "builder": "lead-model",
  "auditor": "auditor-model",
  "budget_usd": 1.0,
  "results": [
    {
      "task": "fix-add",
      "landed": true,
      "accepted": true,
      "usd": 0.01,
      "unpriced": false,
      "billable_tokens": 1000,
      "usd_by_role": { "builder": 0.01 },
      "secs": 1.0,
      "outcome": "t1 — fix add (merged)",
      "calls": 3
    }
  ],
  "skipped": []
}
"#;

/// A provider that answers every request with 401, on a port of its own.
fn refusing_provider() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            // Read the whole request before answering, or the client sees a
            // reset instead of the refusal.
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let Ok(n) = stream.read(&mut buf) else { break };
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
                let Some(head) = text.find("\r\n\r\n") else {
                    continue;
                };
                let length = text[..head]
                    .lines()
                    .filter_map(|l| l.split_once(':'))
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if seen.len() >= head + 4 + length {
                    break;
                }
            }
            let body = r#"{"error":{"message":"no such key","code":401}}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    port
}

/// A home, a folder to run in, a one-task suite and a published run, all
/// in one temporary folder. Nothing of the machine's own is read.
struct Gate {
    dir: TempDir,
    port: u16,
}

impl Gate {
    fn new() -> Self {
        let gate = Gate {
            dir: TempDir::new().expect("tempdir"),
            port: refusing_provider(),
        };
        for d in ["home", "cwd", "pub", "suite/fix-add/repo"] {
            std::fs::create_dir_all(gate.path(d)).expect("mkdir");
        }
        std::fs::write(gate.path("gitconfig"), "").expect("gitconfig");
        std::fs::write(gate.path("suite/fix-add/repo/calc.py"), "def add(a, b):\n")
            .expect("fixture");
        gate.task("");
        gate.crew("auditor-model");
        std::fs::write(gate.path("pub/bench.json"), PUBLISHED).expect("published");
        std::fs::write(gate.path("pub/bench.md"), "# The published run\n").expect("published");
        gate
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    /// The suite's one task, with `needs` as given (a line of TOML, or none).
    fn task(&self, needs: &str) {
        std::fs::write(
            self.path("suite/fix-add/task.toml"),
            format!(
                "title = \"fix add\"\nbrief = \"add(a, b) must return a + b.\"\n\
                 files = [\"calc.py\"]\naccept = [\"exit 0\"]\n{needs}\n"
            ),
        )
        .expect("task");
    }

    /// Lead and builder on `lead-model`, the auditor on `auditor`.
    fn crew(&self, auditor: &str) {
        std::fs::write(
            self.path("home/config.toml"),
            format!(
                "default_connection = \"local\"\n\
                 [connections.local]\nkind = \"openrouter\"\n\
                 base_url = \"http://127.0.0.1:{}/v1\"\napi_backend = \"chat_completions\"\n\
                 env_key = \"RYTER_BENCH_GATE_KEY\"\ndefault_model = \"lead-model\"\n\
                 [orchestrator]\nconnection = \"local\"\nmodel = \"lead-model\"\n\
                 [specialists.auditor]\nconnection = \"local\"\nmodel = \"{auditor}\"\n",
                self.port
            ),
        )
        .expect("config");
    }

    /// `ryter bench --suite <the suite> <args>`: its exit code and all it said.
    fn bench(&self, args: &[&str]) -> (Option<i32>, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_ryter"))
            .arg("bench")
            .arg("--suite")
            .arg(self.path("suite"))
            .args(args)
            .current_dir(self.path("cwd"))
            .env("RYTER_HOME", self.path("home"))
            .env("RYTER_BENCH_GATE_KEY", "not-a-key")
            .env("GIT_CONFIG_GLOBAL", self.path("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // A proxy set on this machine must not get the request.
            .env("NO_PROXY", "127.0.0.1")
            .env("no_proxy", "127.0.0.1")
            .output()
            .expect("run ryter bench");
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code(), said)
    }

    fn stem(&self) -> String {
        self.path("pub/bench").to_string_lossy().into_owned()
    }

    /// The published run is as it was, and nothing was left beside it.
    fn published_is_untouched(&self, said: &str) {
        let read = |p: &Path| std::fs::read_to_string(p).expect("read");
        assert_eq!(read(&self.path("pub/bench.json")), PUBLISHED, "{said}");
        assert_eq!(
            read(&self.path("pub/bench.md")),
            "# The published run\n",
            "{said}"
        );
        let files = std::fs::read_dir(self.path("pub")).expect("pub").count();
        assert_eq!(files, 2, "{said}");
    }
}

/// `--repeat 0` runs nothing. It published "0 of 0" over the published run
/// and exited 0. `--budget-usd 0` is no cap at all.
#[test]
fn arguments_that_measure_nothing_are_refused() {
    let gate = Gate::new();
    let stem = gate.stem();
    for (args, why) in [
        (["--repeat", "0"], "--repeat must be at least 1"),
        (["--budget-usd", "0"], "--budget-usd must be above 0"),
    ] {
        for publish in [&["--publish", stem.as_str()][..], &[]] {
            let (code, said) = gate.bench(&[&args[..], publish].concat());
            assert_eq!(code, Some(1), "{said}");
            assert!(said.contains(why), "{said}");
            gate.published_is_untouched(&said);
        }
    }
}

/// Every seat on one model: the crew won't start. The run stopped, and
/// exited 0.
#[test]
fn a_crew_that_wont_start_fails_the_run() {
    let gate = Gate::new();
    gate.crew("lead-model");
    let stem = gate.stem();
    for args in [&["--publish", stem.as_str()][..], &[]] {
        let (code, said) = gate.bench(args);
        assert_eq!(code, Some(1), "{said}");
        assert!(
            said.contains("no model answered on fix-add (builds paused: "),
            "{said}"
        );
        gate.published_is_untouched(&said);
    }
}

/// A key the provider refuses: every task "failed" with no model ever
/// answering, and the run exited 0. Published where nothing was published
/// yet, it became the run to beat: 0 of 1.
#[test]
fn a_refused_key_fails_the_run() {
    let gate = Gate::new();
    let stem = gate.stem();
    let fresh = gate.path("fresh/bench");
    let fresh_stem = fresh.to_string_lossy().into_owned();
    for args in [
        &["--publish", stem.as_str()][..],
        &["--publish", fresh_stem.as_str()],
        &[],
    ] {
        let (code, said) = gate.bench(args);
        assert_eq!(code, Some(1), "{said}");
        assert!(
            said.contains("no model answered on fix-add (t1 — failed: "),
            "{said}"
        );
        assert!(!said.contains("published "), "{said}");
        gate.published_is_untouched(&said);
    }
    assert!(
        !fresh.with_extension("json").exists() && !fresh.with_extension("md").exists(),
        "a run that measured nothing was published"
    );
}

/// Every task needs something this machine lacks: nothing ran, and the
/// published run was replaced with no results.
#[test]
fn a_suite_this_machine_cant_run_fails_the_run() {
    let gate = Gate::new();
    gate.task("needs = [\"exit 1\"]");
    let stem = gate.stem();
    for args in [&["--publish", stem.as_str()][..], &[]] {
        let (code, said) = gate.bench(args);
        assert_eq!(code, Some(1), "{said}");
        assert!(said.contains("skipped: needs `exit 1`"), "{said}");
        assert!(said.contains("every task was skipped"), "{said}");
        gate.published_is_untouched(&said);
    }
}
