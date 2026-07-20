//! Git backend tests, against plain files rather than Kdenlive projects.
//! The module is editor agnostic and these tests keep it honest about that.

use std::path::{Path, PathBuf};

use cutback::git_backend::Store;

/// A scratch directory that cleans itself up.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!(
            "cutback-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("scratch dir");
        Scratch(base)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store(scratch: &Scratch, name: &str) -> Store {
    Store::open(scratch.path(), &scratch.file(name)).expect("store opens")
}

#[test]
fn first_commit_records_the_file() {
    let s = Scratch::new("first");
    std::fs::write(s.file("p.txt"), b"one").unwrap();

    let store = store(&s, "p.txt");
    let oid = store.commit("first save", "").unwrap();
    assert!(oid.is_some());

    let log = store.log(None).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "first save");
}

/// Kdenlive rewrites the whole document on every save. Without this guard the
/// daemon would pile up commits that record nothing.
#[test]
fn committing_unchanged_content_is_a_no_op() {
    let s = Scratch::new("noop");
    std::fs::write(s.file("p.txt"), b"same").unwrap();
    let store = store(&s, "p.txt");

    assert!(store.commit("first", "").unwrap().is_some());
    assert!(store.commit("second", "").unwrap().is_none());
    assert!(store.commit("third", "").unwrap().is_none());
    assert_eq!(store.log(None).unwrap().len(), 1);
}

#[test]
fn log_is_newest_first() {
    let s = Scratch::new("log");
    let store = store(&s, "p.txt");

    for text in ["a", "b", "c"] {
        std::fs::write(s.file("p.txt"), text).unwrap();
        store.commit(&format!("save {text}"), "").unwrap();
    }

    let log = store.log(None).unwrap();
    let subjects: Vec<&str> = log.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, vec!["save c", "save b", "save a"]);
}

#[test]
fn log_respects_a_limit() {
    let s = Scratch::new("limit");
    let store = store(&s, "p.txt");
    for i in 0..5 {
        std::fs::write(s.file("p.txt"), format!("{i}")).unwrap();
        store.commit(&format!("save {i}"), "").unwrap();
    }
    assert_eq!(store.log(Some(2)).unwrap().len(), 2);
}

#[test]
fn commit_body_survives_the_round_trip() {
    let s = Scratch::new("body");
    std::fs::write(s.file("p.txt"), b"x").unwrap();
    let store = store(&s, "p.txt");
    store
        .commit("subject line", "first detail\nsecond detail")
        .unwrap();

    let log = store.log(None).unwrap();
    assert_eq!(log[0].subject, "subject line");
    assert_eq!(log[0].body, "first detail\nsecond detail");
}

/// The hard requirement: a restore returns the exact bytes that were committed.
/// The content here is chosen to catch the ways this usually breaks, CRLF
/// endings being normalized, a missing trailing newline being added, trailing
/// whitespace being stripped, and non UTF-8 bytes being mangled.
#[test]
fn restore_is_byte_for_byte() {
    let s = Scratch::new("bytes");
    let awkward: &[u8] = b"<mlt>\r\n  <profile/>\t \r\n</mlt>  \x00\xff\xfe no trailing newline";

    std::fs::write(s.file("p.kdenlive"), awkward).unwrap();
    let store = store(&s, "p.kdenlive");
    store.commit("save", "").unwrap();

    // Overwrite with something completely different, then restore.
    std::fs::write(s.file("p.kdenlive"), b"clobbered").unwrap();
    store.restore("HEAD").unwrap();

    let back = std::fs::read(s.file("p.kdenlive")).unwrap();
    assert_eq!(
        back, awkward,
        "restored file must match the committed bytes"
    );
}

#[test]
fn restore_reaches_back_past_the_latest_commit() {
    let s = Scratch::new("history");
    let store = store(&s, "p.txt");

    std::fs::write(s.file("p.txt"), b"version one").unwrap();
    store.commit("one", "").unwrap();
    std::fs::write(s.file("p.txt"), b"version two").unwrap();
    store.commit("two", "").unwrap();

    let log = store.log(None).unwrap();
    store.restore(&log[1].id).unwrap();
    assert_eq!(std::fs::read(s.file("p.txt")).unwrap(), b"version one");
}

#[test]
fn file_at_reads_without_touching_disk() {
    let s = Scratch::new("readonly");
    let store = store(&s, "p.txt");

    std::fs::write(s.file("p.txt"), b"old").unwrap();
    store.commit("old", "").unwrap();
    std::fs::write(s.file("p.txt"), b"new").unwrap();
    store.commit("new", "").unwrap();

    let log = store.log(None).unwrap();
    assert_eq!(store.file_at(&log[1].id).unwrap(), b"old");
    // The working file is untouched by a read.
    assert_eq!(std::fs::read(s.file("p.txt")).unwrap(), b"new");
}

#[test]
fn branch_and_checkout_switch_the_file() {
    let s = Scratch::new("branch");
    let store = store(&s, "p.txt");

    std::fs::write(s.file("p.txt"), b"main work").unwrap();
    store.commit("on main", "").unwrap();
    let main = store.current_branch().unwrap();

    store.branch("experiment").unwrap();
    store.checkout("experiment").unwrap();
    assert_eq!(store.current_branch().unwrap(), "experiment");

    std::fs::write(s.file("p.txt"), b"experimental work").unwrap();
    store.commit("on experiment", "").unwrap();

    store.checkout(&main).unwrap();
    assert_eq!(
        std::fs::read(s.file("p.txt")).unwrap(),
        b"main work",
        "checking out the original branch restores its content"
    );

    store.checkout("experiment").unwrap();
    assert_eq!(
        std::fs::read(s.file("p.txt")).unwrap(),
        b"experimental work"
    );
}

#[test]
fn branches_are_listed() {
    let s = Scratch::new("branches");
    let store = store(&s, "p.txt");
    std::fs::write(s.file("p.txt"), b"x").unwrap();
    store.commit("first", "").unwrap();

    store.branch("alt").unwrap();
    let names = store.branches().unwrap();
    assert!(names.contains(&"alt".to_string()), "{names:?}");
}

#[test]
fn reopening_an_existing_store_keeps_history() {
    let s = Scratch::new("reopen");
    {
        let store = store(&s, "p.txt");
        std::fs::write(s.file("p.txt"), b"x").unwrap();
        store.commit("first", "").unwrap();
    }
    let store = store(&s, "p.txt");
    assert_eq!(store.log(None).unwrap().len(), 1);
}

#[test]
fn the_store_does_not_create_a_dot_git_directory() {
    let s = Scratch::new("nodotgit");
    let store = store(&s, "p.txt");
    std::fs::write(s.file("p.txt"), b"x").unwrap();
    store.commit("first", "").unwrap();

    // A project folder that is already a git repository has to keep working,
    // so the store must stay out of .git entirely.
    assert!(!s.file(".git").exists());
    assert!(s.file(".cutback").is_dir());
}

#[test]
fn unknown_revisions_and_branches_are_errors() {
    let s = Scratch::new("errors");
    let store = store(&s, "p.txt");
    std::fs::write(s.file("p.txt"), b"x").unwrap();
    store.commit("first", "").unwrap();

    assert!(store.file_at("nope").is_err());
    assert!(store.checkout("no-such-branch").is_err());
}

#[test]
fn branching_before_any_commit_fails_clearly() {
    let s = Scratch::new("empty");
    let store = store(&s, "p.txt");
    let err = store.branch("x").unwrap_err().to_string();
    assert!(err.contains("before the first save"), "{err}");
}

#[test]
fn log_on_an_empty_store_is_empty() {
    let s = Scratch::new("emptylog");
    let store = store(&s, "p.txt");
    assert!(store.log(None).unwrap().is_empty());
}
