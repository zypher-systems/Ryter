use super::*;
use crate::git::{git, init_repo};
use crate::session::{Session, spend_record};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct Fixture {
    _home: TempDir,
    _repo: TempDir,
    home: PathBuf,
    repo: PathBuf,
}

fn fixture() -> Fixture {
    let home = TempDir::new().unwrap();
    let repo = TempDir::new().unwrap();
    init_repo(repo.path()).unwrap();
    Fixture {
        home: home.path().canonicalize().unwrap(),
        repo: repo.path().canonicalize().unwrap(),
        _home: home,
        _repo: repo,
    }
}

impl Fixture {
    fn session(&self) -> Session {
        Session::create(&self.home, &self.repo, "c".into(), "m".into()).unwrap()
    }

    /// A checkpoint ref made `days` ago, holding a file of its own.
    fn old_ref(&self, name: &str, days: u64, file: &str) -> String {
        std::fs::write(self.repo.join(file), format!("{name} {days}\n").repeat(50)).unwrap();
        let blob = git(&self.repo, &["hash-object", "-w", file])
            .unwrap()
            .trim()
            .to_string();
        std::fs::remove_file(self.repo.join(file)).unwrap();
        let tree = git_stdin(
            &self.repo,
            &["mktree"],
            &format!("100644 blob {blob}\t{file}\n"),
        )
        .unwrap()
        .trim()
        .to_string();
        let when = (now_ms() / 1000).saturating_sub(days * 86_400);
        let out = Command::new("git")
            .args(["-C", &self.repo.to_string_lossy()])
            .args(["commit-tree", &tree, "-m", "ryter undo checkpoint"])
            .env("GIT_COMMITTER_DATE", format!("{when} +0000"))
            .env("GIT_AUTHOR_DATE", format!("{when} +0000"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let sha = String::from_utf8(out.stdout).unwrap().trim().to_string();
        git(
            &self.repo,
            &["update-ref", &format!("{PREFIX}{name}"), &sha],
        )
        .unwrap();
        sha
    }

    fn refs(&self) -> Vec<String> {
        git(
            &self.repo,
            &["for-each-ref", "--format=%(refname)", "refs/ryter/"],
        )
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
    }

    fn report(&self) -> Report {
        report(&self.home, &self.repo, now_ms()).unwrap()
    }
}

/// Set a session's last use `days` ago, as its index says it.
fn age(s: &Session, days: u64) {
    let path = s.dir.join("meta.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["updated_at"] = serde_json::Value::String((now_ms() - days * DAY_MS).to_string());
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
}

fn hold_of(r: &Report, short: &str) -> Hold {
    r.refs()
        .iter()
        .find(|c| c.name == format!("{PREFIX}{short}"))
        .unwrap_or_else(|| panic!("no ref {short}"))
        .hold
}

#[test]
fn a_checkpoint_a_session_names_stays_and_one_none_names_can_go() {
    let f = fixture();
    let mut s = f.session();
    let id = s.meta.id.to_string();
    let named = f.old_ref(&format!("{id}-1"), 10, "a.txt");
    s.push_checkpoint(named).unwrap();
    f.old_ref(&format!("{id}-0"), 10, "b.txt");
    f.old_ref("audit-after-0123456789ab", 10, "c.txt");
    crate::git::checkpoint(&f.repo, &format!("{id}-2")).unwrap();
    crate::git::checkpoint(&f.repo, "review").unwrap();
    git(&f.repo, &["tag", "v1"]).unwrap();
    git(&f.repo, &["branch", "side"]).unwrap();

    let r = f.report();
    assert_eq!(hold_of(&r, &format!("{id}-1")), Hold::InUse);
    assert_eq!(hold_of(&r, &format!("{id}-0")), Hold::Free);
    assert_eq!(hold_of(&r, "audit-after-0123456789ab"), Hold::Free);
    assert_eq!(hold_of(&r, &format!("{id}-2")), Hold::Recent);
    assert_eq!(hold_of(&r, "review"), Hold::Working);

    let p = plan(&r, Cleanup::Unreachable, Some(&id));
    let mut names: Vec<_> = p.checkpoints.iter().map(|c| c.name.clone()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            format!("{PREFIX}{id}-0"),
            format!("{PREFIX}audit-after-0123456789ab")
        ]
    );
    assert!(p.sessions.is_empty());
    let said = p.explain().join("\n");
    assert!(said.contains("Removes 2 checkpoints"), "{said}");
    assert!(said.contains("1 checkpoint a session reaches"), "{said}");
    assert!(said.contains("made in the last day"), "{said}");
    assert!(said.contains("git gc"), "{said}");

    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!(done.checkpoints, 2, "{done:?}");
    let left = f.refs();
    assert!(left.contains(&format!("{PREFIX}{id}-1")));
    assert!(left.contains(&format!("{PREFIX}{id}-2")));
    assert!(left.contains(&format!("{PREFIX}review")));
    assert_eq!(left.len(), 3, "{left:?}");
    // Branches and tags are not Ryter's.
    assert!(git(&f.repo, &["rev-parse", "side", "v1"]).is_ok());
    // The session's own undo still has its commit.
    assert!(Session::open(&s.dir).is_ok());
}

/// Deleting a session removes its folder and leaves its checkpoints, which
/// the cleanup then finds: two steps, each tested alone.
#[test]
fn a_deleted_session_leaves_its_checkpoints_for_the_cleanup() {
    let f = fixture();
    let mut s = f.session();
    let id = s.meta.id.to_string();
    let sha = f.old_ref(&format!("{id}-1"), 10, "a.txt");
    s.push_checkpoint(sha).unwrap();
    assert_eq!(hold_of(&f.report(), &format!("{id}-1")), Hold::InUse);

    Session::remove(&s.dir).unwrap();
    assert_eq!(f.refs(), [format!("{PREFIX}{id}-1")]);
    let r = f.report();
    assert!(r.sessions.is_empty());
    assert_eq!(hold_of(&r, &format!("{id}-1")), Hold::Free);
    assert_eq!(plan(&r, Cleanup::Unreachable, None).checkpoints.len(), 1);
}

#[test]
fn old_sessions_go_with_the_checkpoints_only_they_reach() {
    let f = fixture();
    let mut old = f.session();
    let mut kept = f.session();
    let mut open = f.session();
    let old_id = old.meta.id.to_string();
    let only = f.old_ref(&format!("{old_id}-1"), 40, "a.txt");
    let shared = f.old_ref(&format!("{old_id}-2"), 40, "b.txt");
    old.push_checkpoint(only).unwrap();
    old.push_checkpoint(shared.clone()).unwrap();
    // A redo of the other session's names the same commit.
    kept.push_checkpoint(shared).unwrap();
    old.record_spend(spend_record(
        "c".into(),
        "m".into(),
        crate::Role::Build,
        Default::default(),
        Some(1.25),
    ))
    .unwrap();
    std::fs::create_dir_all(crate::page::dir(&f.home, &old_id)).unwrap();
    std::fs::write(crate::page::dir(&f.home, &old_id).join("p.html"), "x").unwrap();
    let before = crate::project::project_spend(&f.home, &f.repo).unwrap();
    age(&old, 40);
    age(&kept, 2);
    // The open session is left whatever its age.
    age(&open, 400);
    open.set_title("open").ok();
    age(&open, 400);

    let r = f.report();
    let p = plan(&r, Cleanup::OlderThan(30), Some(open.meta.id.as_str()));
    assert_eq!(
        p.sessions.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [old_id.as_str()]
    );
    assert_eq!(
        p.checkpoints
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        [format!("{PREFIX}{old_id}-1")]
    );
    let said = p.explain().join("\n");
    assert!(said.contains("Removes 1 session"), "{said}");
    assert!(said.contains("keeps the $1.25"), "{said}");
    assert!(said.contains("this session"), "{said}");

    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!((done.sessions, done.checkpoints), (1, 1), "{done:?}");
    assert!(!old.dir.exists());
    assert!(!crate::page::dir(&f.home, &old_id).exists());
    assert!(kept.dir.exists() && open.dir.exists());
    assert_eq!(f.refs(), [format!("{PREFIX}{old_id}-2")]);
    let after = crate::project::project_spend(&f.home, &f.repo).unwrap();
    assert_eq!(after.total_usd, before.total_usd);
}

#[test]
fn a_session_used_since_the_preview_is_left() {
    let f = fixture();
    let s = f.session();
    age(&s, 40);
    let p = plan(&f.report(), Cleanup::OlderThan(30), None);
    assert_eq!(p.sessions.len(), 1);
    age(&s, 0);
    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!(done.sessions, 0);
    assert!(s.dir.exists());
    assert!(
        done.summary().contains("used since the preview"),
        "{done:?}"
    );
}

#[test]
fn a_checkpoint_a_session_came_to_name_after_the_preview_is_left() {
    let f = fixture();
    let mut s = f.session();
    let sha = f.old_ref("orphan", 10, "a.txt");
    let p = plan(&f.report(), Cleanup::Unreachable, None);
    assert_eq!(p.checkpoints.len(), 1);
    s.push_checkpoint(sha).unwrap();
    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!(done.checkpoints, 0);
    assert_eq!(f.refs(), [format!("{PREFIX}orphan")]);
    assert!(done.summary().contains("came to use them"), "{done:?}");
}

#[test]
fn a_checkpoint_that_moved_after_the_preview_is_left() {
    let f = fixture();
    f.old_ref("orphan", 10, "a.txt");
    let p = plan(&f.report(), Cleanup::Unreachable, None);
    f.old_ref("orphan", 12, "b.txt");
    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!(done.checkpoints, 0);
    assert_eq!(f.refs(), [format!("{PREFIX}orphan")]);
}

/// A session whose index can't be read might name any checkpoint without
/// an owner, and those it owns: they stay.
#[test]
fn an_unreadable_index_holds_what_it_might_name() {
    let f = fixture();
    let s = f.session();
    let id = s.meta.id.to_string();
    let other = f.session();
    let other_id = other.meta.id.to_string();
    f.old_ref(&format!("{id}-1"), 10, "a.txt");
    f.old_ref(&format!("{other_id}-1"), 10, "b.txt");
    f.old_ref("audit-after-0123456789ab", 10, "c.txt");
    std::fs::write(s.dir.join("meta.json"), "{ torn").unwrap();
    let r = f.report();
    assert_eq!(hold_of(&r, &format!("{id}-1")), Hold::InUse);
    assert_eq!(hold_of(&r, "audit-after-0123456789ab"), Hold::InUse);
    assert_eq!(hold_of(&r, &format!("{other_id}-1")), Hold::Free);
}

/// A newer Ryter's index may name a checkpoint in a field this one does not
/// know; every object id in it counts, and a newer schema holds as unread.
#[test]
fn a_newer_index_holds_what_it_might_name() {
    let f = fixture();
    let s = f.session();
    let sha = f.old_ref("orphan", 10, "a.txt");
    let other = f.old_ref("audit-after-0123456789ab", 10, "b.txt");
    let path = s.dir.join("meta.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["a_later_field"] = serde_json::json!({ "deep": [sha] });
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = f.report();
    assert_eq!(hold_of(&r, "orphan"), Hold::InUse);
    assert_eq!(hold_of(&r, "audit-after-0123456789ab"), Hold::Free);
    v["schema"] = serde_json::json!(crate::schema::SCHEMA + 1);
    std::fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert_eq!(
        hold_of(&f.report(), "audit-after-0123456789ab"),
        Hold::InUse,
        "{other}"
    );
}

/// The same repository opened from another folder (a linked worktree)
/// shares its checkpoints: a session there still holds what it names.
#[test]
fn a_session_of_another_folder_holds_what_it_names() {
    let f = fixture();
    let elsewhere = TempDir::new().unwrap();
    let mut s = Session::create(&f.home, elsewhere.path(), "c".into(), "m".into()).unwrap();
    let sha = f.old_ref("theirs", 10, "a.txt");
    s.push_checkpoint(sha).unwrap();
    let r = f.report();
    assert!(r.sessions.is_empty());
    assert_eq!(r.all_sessions, 1);
    assert_eq!(hold_of(&r, "theirs"), Hold::InUse);
}

#[test]
fn recovery_copies_are_listed_and_only_they_go() {
    let f = fixture();
    let s = f.session();
    let copy = s.dir.join("transcript.jsonl.recovery-x.bak");
    std::fs::write(&copy, "old\n").unwrap();
    std::fs::write(s.dir.join("notes").join("x.bak"), "mine").unwrap();
    let r = f.report();
    assert_eq!(r.copies.len(), 1);
    let p = plan(&r, Cleanup::Copies, None);
    assert!(p.checkpoints.is_empty() && p.sessions.is_empty());
    assert!(p.explain()[0].contains("1 recovery copy (4 B)"), "{p:?}");
    let done = apply(&f.home, &p, now_ms()).unwrap();
    assert_eq!(done.copies, 1);
    assert!(!copy.exists());
    assert!(s.dir.join("transcript.jsonl").exists());
    assert!(s.dir.join("notes").join("x.bak").exists());
}

#[test]
fn outside_a_repository_there_are_no_checkpoints() {
    let home = TempDir::new().unwrap();
    let dir = TempDir::new().unwrap();
    let r = report(home.path(), dir.path(), now_ms()).unwrap();
    assert!(r.checkpoints.is_none());
    let p = plan(&r, Cleanup::Unreachable, None);
    assert!(p.is_empty());
    assert_eq!(p.amount(), "nothing");
}

#[test]
fn the_space_only_checkpoints_hold_is_counted_when_git_can() {
    let f = fixture();
    f.old_ref("orphan", 10, "a.txt");
    let can = git(&f.repo, &["rev-list", "--disk-usage", "HEAD"]).is_ok();
    let p = plan(&f.report(), Cleanup::Unreachable, None);
    if can {
        assert!(p.checkpoint_bytes.is_some_and(|b| b > 0), "{p:?}");
    } else {
        assert!(p.explain().join(" ").contains("can't say how much"));
    }
}

#[test]
fn names_and_sizes() {
    let id = "019a3b4c-5d6e-7f80-9a1b-2c3d4e5f6a7b";
    assert_eq!(owner_of(&format!("{id}-3")).as_deref(), Some(id));
    assert_eq!(owner_of(&format!("audit-{id}-2")).as_deref(), Some(id));
    assert_eq!(
        owner_of(&format!("{id}-after-0123456789ab")).as_deref(),
        Some(id)
    );
    assert_eq!(owner_of("audit-after-0123456789ab"), None);
    assert_eq!(owner_of("review"), None);
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(3 * 1024 + 400), "3.4 KB");
    assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    assert_eq!(count(1, "recovery copy"), "1 recovery copy");
    assert_eq!(count(2, "recovery copy"), "2 recovery copies");
    assert_eq!(count(3, "checkpoint"), "3 checkpoints");
}
