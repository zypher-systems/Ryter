//! Every kept release's files, read by today's code: what 1.0 writes, 1.x
//! reads. `tests/write_fixture.rs` writes a folder under `fixtures/persisted/`
//! at a release; a folder is never rewritten, and a test here failing means
//! today's code can no longer read what somebody has on disk.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ryter_core::config;
use ryter_core::event::AgentEvent;
use ryter_core::llm::{Message, MessagesBlock};
use ryter_core::role::Role;
use ryter_core::run::{self, Found, RunFile};
use ryter_core::schema::SCHEMA;
use ryter_core::session::{Session, SpendRecord};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A field renamed since a release, as `(kept name, name written now)`: the
/// reader must take both, and an entry here is how a rename says it did.
const RENAMED: &[(&str, &str)] = &[];

const CWD: &str = "/work/fixture";

fn kept() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/persisted");
    let mut releases: Vec<PathBuf> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    releases.sort();
    releases
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A release's files somewhere they can be opened and changed.
struct Release {
    _dir: TempDir,
    name: String,
    home: PathBuf,
    project: PathBuf,
}

impl Release {
    fn new(kept: &Path) -> Self {
        let dir = TempDir::new().unwrap();
        copy_tree(kept, dir.path());
        Self {
            home: dir.path().join("home"),
            project: dir.path().join("project"),
            name: kept.file_name().unwrap().to_string_lossy().into_owned(),
            _dir: dir,
        }
    }

    fn session_dir(&self) -> PathBuf {
        let list = Session::list(&self.home, Path::new(CWD)).unwrap();
        assert_eq!(list.len(), 1, "{}", self.name);
        list[0].dir.clone()
    }

    fn session(&self) -> Session {
        Session::open(&self.session_dir()).unwrap()
    }
}

fn each_release(check: impl Fn(&Release)) {
    for kept in kept() {
        check(&Release::new(&kept));
    }
}

/// Today's mark is proven by a folder of its own, and the releases from
/// before there was a mark by theirs.
#[test]
fn the_first_release_and_the_first_marked_are_kept() {
    let names: Vec<String> = kept()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.iter().any(|n| n == "0.42.0"), "{names:?}");
    assert!(names.iter().any(|n| n == "schema-1"), "{names:?}");
}

#[test]
fn every_kept_release_opens_with_everything_in_it() {
    each_release(|r| {
        let name = &r.name;
        let session = r.session();
        let meta = &session.meta;
        assert_eq!(meta.title, "The kettle number", "{name}");
        assert_eq!(meta.model, "claude-opus-5-5", "{name}");
        assert_eq!(meta.mode, Some(Role::Plan), "{name}");
        assert_eq!(meta.spend_usd_total, Some(0.75), "{name}");
        assert!(meta.spend_unknown && meta.spend_incomplete, "{name}");
        assert!(meta.is_unpriced("no-price"), "{name}");
        assert_eq!(meta.spend_label(), "≥$0.75", "{name}");
        assert_eq!(meta.checkpoints, ["0123456789abcdef"], "{name}");
        assert_eq!(meta.turn_checkpoint.as_deref(), Some("0123456789abcdef"));
        assert_eq!(
            meta.turn_records["0123456789abcdef"].after.as_deref(),
            Some("fedcba9876543210"),
            "{name}"
        );
        assert!(session.recovery_notices.is_empty(), "{name}");

        let [user, asked, result, answer] = session.transcript.as_slice() else {
            panic!("{name}: {:?}", session.transcript);
        };
        assert_eq!(user.role, "user", "{name}");
        let calls = asked.tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].arguments, r#"{"path":"notes.txt"}"#, "{name}");
        let signed = asked.messages_blocks.as_ref().unwrap().iter().any(
            |b| matches!(b, MessagesBlock::Thinking { signature, .. } if signature == "sig_1"),
        );
        assert!(signed, "{name}");
        assert_eq!(result.tool_call_id.as_deref(), Some("call_1"), "{name}");
        assert_eq!(answer.content, "The number is 7431.", "{name}");

        let rows = session.spend_log().unwrap();
        assert_eq!(rows.len(), 3, "{name}");
        assert_eq!(rows[0].total_usd, Some(0.25), "{name}");
        assert_eq!(rows[0].cached_tokens, 800, "{name}");
        assert_eq!(rows[0].cache_write_tokens, 50, "{name}");
        assert_eq!(rows[1].total_usd, None, "{name}");
        assert!(rows[2].incomplete && rows[2].scout, "{name}");
        assert_eq!(rows[2].host.as_deref(), Some("Host"), "{name}");

        let events = std::fs::read_to_string(session.dir.join("events.jsonl")).unwrap();
        let events: Vec<AgentEvent> = events
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(events.len() >= 2, "{name}");
    });
}

#[test]
fn every_kept_release_configures_ryter() {
    each_release(|r| {
        let name = &r.name;
        let cfg = config::load_at(&r.home, None, false).unwrap();
        assert_eq!(cfg.spend.session_budget_usd, 5.0, "{name}");
        assert_eq!(cfg.limits.rounds, 40, "{name}");
        assert_eq!(cfg.sandbox.profile, "workspace", "{name}");
        assert!(!cfg.features.web, "{name}");
        let routing = config::load_routing(&r.home);
        assert_eq!(routing["openrouter"].order, ["baseten"], "{name}");
        let live = cfg.connections["openrouter"].routing.as_ref().unwrap();
        assert_eq!(live.order, ["baseten"], "{name}");
    });
}

#[test]
fn every_kept_release_keeps_its_project_files() {
    each_release(|r| {
        let name = &r.name;
        let Found::Unapproved(found, _) = run::find(&r.project, &r.home) else {
            panic!("{name}: the run file did not load");
        };
        assert_eq!(found.start.as_deref(), Some("python3 -m http.server 8000"));
        assert_eq!(found.test, ["make test"], "{name}");
        assert_eq!(
            ryter_core::plan::on_record(&r.project).as_deref(),
            Some(".ryter/plans/2026-10-30-kettle.md"),
            "{name}"
        );
        let plan = std::fs::read_to_string(r.project.join(".ryter/plan.md")).unwrap();
        assert!(plan.contains("Read the number."), "{name}");
    });
}

/// Every leaf of `value`, by its path.
fn leaves(value: &Value, at: &str, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                leaves(value, &format!("{at}.{key}"), out);
            }
        }
        Value::Array(items) => {
            for (i, value) in items.iter().enumerate() {
                leaves(value, &format!("{at}.{i}"), out);
            }
        }
        leaf => {
            out.insert(at.to_string(), leaf.clone());
        }
    }
}

/// Saved again, nothing a release wrote has gone or changed. This is what
/// makes a change an addition: a field dropped is a failure here, and a field
/// renamed is one until it reads both names and is listed in [`RENAMED`].
fn nothing_lost(release: &str, file: &str, kept: &Value, saved: &Value) {
    let (mut before, mut after) = (BTreeMap::new(), BTreeMap::new());
    leaves(kept, "", &mut before);
    leaves(saved, "", &mut after);
    for (path, value) in before {
        let renamed = RENAMED
            .iter()
            .fold(path.clone(), |p, (old, new)| p.replace(old, new));
        assert_eq!(
            after.get(&renamed),
            Some(&value),
            "{release}/{file}: {path} was lost or changed"
        );
    }
}

fn toml_json(text: &str) -> Value {
    serde_json::to_value(toml::from_str::<toml::Value>(text).unwrap()).unwrap()
}

fn jsonl_lost_nothing<T>(r: &Release, dir: &Path, file: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let text = std::fs::read_to_string(dir.join(file)).unwrap();
    for line in text.lines() {
        let row: T = serde_json::from_str(line).unwrap();
        let kept: Value = serde_json::from_str(line).unwrap();
        nothing_lost(&r.name, file, &kept, &serde_json::to_value(&row).unwrap());
    }
}

#[test]
fn what_a_release_wrote_survives_being_saved_again() {
    each_release(|r| {
        let dir = r.session_dir();
        let meta = std::fs::read_to_string(dir.join("meta.json")).unwrap();
        let kept: Value = serde_json::from_str(&meta).unwrap();
        let session = Session::open(&dir).unwrap();
        let saved = serde_json::to_value(&session.meta).unwrap();
        nothing_lost(&r.name, "meta.json", &kept, &saved);
        jsonl_lost_nothing::<SpendRecord>(r, &dir, "spend.jsonl");
        jsonl_lost_nothing::<Message>(r, &dir, "transcript.jsonl");

        let fresh = TempDir::new().unwrap();
        let cfg = config::load_at(&r.home, None, false).unwrap();
        config::save_settings(fresh.path(), &cfg).unwrap();
        config::save_routing(fresh.path(), &config::load_routing(&r.home)).unwrap();
        for file in ["settings.toml", "routing.toml"] {
            let kept = std::fs::read_to_string(r.home.join(file)).unwrap();
            let saved = std::fs::read_to_string(fresh.path().join(file)).unwrap();
            nothing_lost(&r.name, file, &toml_json(&kept), &toml_json(&saved));
        }

        let text = std::fs::read_to_string(r.project.join(run::FILE)).unwrap();
        let saved = toml::from_str::<RunFile>(&text).unwrap().text();
        nothing_lost(&r.name, "run.toml", &toml_json(&text), &toml_json(&saved));
    });
}

/// The folder from the first marked release carries the mark everywhere it
/// is promised.
#[test]
fn a_marked_release_is_marked_everywhere() {
    let root = kept()
        .into_iter()
        .find(|p| p.ends_with("schema-1"))
        .unwrap();
    let r = Release::new(&root);
    let dir = r.session_dir();
    let marked = |text: &str, file: &str| {
        assert_eq!(ryter_core::schema::of_toml(text), 1, "{file}");
        assert!(text.contains("schema = 1"), "{file}: {text}");
    };
    for file in ["config.toml", "settings.toml", "routing.toml"] {
        marked(&std::fs::read_to_string(r.home.join(file)).unwrap(), file);
    }
    marked(
        &std::fs::read_to_string(r.project.join(run::FILE)).unwrap(),
        "run.toml",
    );
    let meta: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["schema"], json!(1));
    let spend = std::fs::read_to_string(dir.join("spend.jsonl")).unwrap();
    for row in spend.lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        assert_eq!(row["schema"], json!(1), "{row}");
    }
    let plan = std::fs::read_to_string(r.project.join(".ryter/plan.md")).unwrap();
    assert!(plan.contains("schema 1"), "{plan}");
}

fn with_newer_mark(text: &str) -> String {
    let mark = format!("schema = {}", SCHEMA + 1);
    if text.contains("schema = 1") {
        text.replace("schema = 1", &mark)
    } else {
        format!("{mark}\n{text}")
    }
}

/// An older Ryter saving a newer session would drop what it can't read, so
/// nothing is touched: not the meta, and not a torn log it would repair.
#[test]
fn a_newer_session_is_refused_and_left_as_it_was() {
    each_release(|r| {
        let dir = r.session_dir();
        let mut meta: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("meta.json")).unwrap()).unwrap();
        meta["schema"] = json!(SCHEMA + 1);
        meta["from_the_future"] = json!(true);
        let written = meta.to_string();
        std::fs::write(dir.join("meta.json"), &written).unwrap();
        let log = dir.join("spend.jsonl");
        let mut torn = std::fs::read_to_string(&log).unwrap();
        torn.push_str("{\"ts\":");
        std::fs::write(&log, &torn).unwrap();

        let Err(why) = Session::open(&dir) else {
            panic!("{}: opened", r.name);
        };
        assert!(why.to_string().contains("newer Ryter"), "{why}");
        assert_eq!(
            std::fs::read_to_string(dir.join("meta.json")).unwrap(),
            written
        );
        assert_eq!(std::fs::read_to_string(&log).unwrap(), torn);
    });
}

#[test]
fn a_newer_file_stops_ryter_before_a_save_can_replace_it() {
    each_release(|r| {
        for file in ["config.toml", "settings.toml", "routing.toml"] {
            let path = r.home.join(file);
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, with_newer_mark(&text)).unwrap();
            let Err(why) = config::load_at(&r.home, None, false) else {
                panic!("{}/{file}: loaded", r.name);
            };
            assert!(why.to_string().contains(file), "{why}");
            assert!(why.to_string().contains("newer Ryter"), "{why}");
            std::fs::write(&path, text).unwrap();
        }
        assert!(config::load_at(&r.home, None, false).is_ok());
    });
}

#[test]
fn a_newer_run_file_is_broken_and_says_why() {
    each_release(|r| {
        let path = r.project.join(run::FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, with_newer_mark(&text)).unwrap();
        let Found::Broken(why) = run::find(&r.project, &r.home) else {
            panic!("{}: read a newer run file", r.name);
        };
        assert!(why.contains("newer Ryter"), "{why}");
    });
}

/// A connection may be called anything, and `schema` is a word.
#[test]
fn a_connection_named_schema_keeps_its_rules() {
    let home = TempDir::new().unwrap();
    let rules: config::RoutingConfig = toml::from_str("order = [\"baseten\"]\n").unwrap();
    let saved = BTreeMap::from([
        ("schema".to_string(), rules.clone()),
        ("openrouter".to_string(), rules),
    ]);
    config::save_routing(home.path(), &saved).unwrap();
    assert_eq!(config::load_routing(home.path()), saved);
    assert!(config::load_at(home.path(), None, false).is_ok());
}

/// The mark a file gains leaves the rules that were there, on a save and on
/// the load after it.
#[test]
fn the_routing_file_reads_the_same_with_its_mark() {
    let home = TempDir::new().unwrap();
    let rules: config::RoutingConfig = toml::from_str("order = [\"baseten\"]\n").unwrap();
    let saved = BTreeMap::from([("openrouter".to_string(), rules)]);
    config::save_routing(home.path(), &saved).unwrap();
    let text = std::fs::read_to_string(home.path().join("routing.toml")).unwrap();
    assert!(text.contains("schema = 1"), "{text}");
    assert_eq!(config::load_routing(home.path()), saved);
    let cfg = config::load_at(home.path(), None, false).unwrap();
    assert!(cfg.connections["openrouter"].routing.is_some());
    assert!(!cfg.connections.contains_key("schema"));
}
