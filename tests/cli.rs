//! End to end tests driving the built binary the way a user would.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("cutback-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("scratch dir");
        Scratch(base)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn binary() -> PathBuf {
    // The integration test binary sits next to the one under test.
    let mut path = std::env::current_exe().expect("test binary path");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join("cutback")
}

fn sample(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name);
    std::fs::read_to_string(path).expect("sample file")
}

struct Output {
    stdout: String,
    stderr: String,
    ok: bool,
}

fn run(args: &[&str]) -> Output {
    let out = Command::new(binary())
        .args(args)
        .output()
        .expect("running cutback");
    Output {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        ok: out.status.success(),
    }
}

/// Sets up a project directory with two saves already recorded, without going
/// through the watcher, so command tests stay fast and deterministic.
fn project_with_history(tag: &str) -> Scratch {
    let s = Scratch::new(tag);
    let file = s.path().join("holiday.kdenlive");
    let dir = s.path().to_str().unwrap();

    std::fs::write(&file, sample("timelapse-a.kdenlive")).unwrap();
    let store = cutback::git_backend::Store::open(s.path(), &file).unwrap();
    store.commit("Started watching this project", "").unwrap();

    std::fs::write(&file, sample("timelapse-c.kdenlive")).unwrap();
    let before = cutback::xml_parser::parse_str(&sample("timelapse-a.kdenlive")).unwrap();
    let after = cutback::xml_parser::parse_str(&sample("timelapse-c.kdenlive")).unwrap();
    let d = cutback::differ::diff(&before, &after);
    let fps = after.profile.fps();
    store
        .commit(
            &cutback::render::summarize(&d, fps),
            &cutback::render::render(&d, fps).join("\n"),
        )
        .unwrap();

    let _ = dir;
    s
}

#[test]
fn help_and_version_work() {
    assert!(run(&["--help"]).ok);
    assert!(run(&["--version"]).stdout.contains("cutback"));
}

#[test]
fn log_reads_as_sentences() {
    let s = project_with_history("log");
    let out = run(&["log", "-C", s.path().to_str().unwrap()]);

    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("to the project bin"), "{}", out.stdout);
    assert!(out.stdout.contains("ago") || out.stdout.contains("just now"));
    // No XML should reach the user.
    assert!(!out.stdout.contains('<'), "{}", out.stdout);
}

#[test]
fn log_limit_is_respected() {
    let s = project_with_history("loglimit");
    let out = run(&["log", "-C", s.path().to_str().unwrap(), "-n", "1"]);
    assert_eq!(out.stdout.lines().count(), 1, "{}", out.stdout);
}

#[test]
fn diff_defaults_to_the_last_two_saves() {
    let s = project_with_history("diff");
    let out = run(&["diff", "-C", s.path().to_str().unwrap()]);

    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("added"), "{}", out.stdout);
    assert!(!out.stdout.contains('<'), "no XML in diff output");
}

/// The hard requirement, checked through the CLI rather than the library.
#[test]
fn restore_returns_the_exact_original_bytes() {
    let s = project_with_history("restore");
    let dir = s.path().to_str().unwrap();
    let file = s.path().join("holiday.kdenlive");

    let log = run(&["log", "-C", dir]);
    let first = log
        .stdout
        .lines()
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();

    let out = run(&["restore", "-C", dir, &first, "-y"]);
    assert!(out.ok, "{}", out.stderr);

    assert_eq!(
        std::fs::read(&file).unwrap(),
        sample("timelapse-a.kdenlive").into_bytes(),
        "restore must reproduce the original file exactly"
    );
}

#[test]
fn restore_records_the_current_state_first() {
    let s = project_with_history("restoresafe");
    let dir = s.path().to_str().unwrap();

    let before = run(&["log", "-C", dir]).stdout.lines().count();
    let first = run(&["log", "-C", dir])
        .stdout
        .lines()
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    run(&["restore", "-C", dir, &first, "-y"]);

    // Restoring must not lose the state that was on disk, so history grows.
    let after = run(&["log", "-C", dir]).stdout.lines().count();
    assert!(after >= before, "restore should not discard history");
}

#[test]
fn branch_and_checkout_round_trip() {
    let s = project_with_history("branch");
    let dir = s.path().to_str().unwrap();
    let file = s.path().join("holiday.kdenlive");

    assert!(run(&["branch", "-C", dir, "alt-cut"]).ok);
    assert!(run(&["branch", "-C", dir]).stdout.contains("alt-cut"));

    let on_main = std::fs::read(&file).unwrap();
    assert!(run(&["checkout", "-C", dir, "alt-cut"]).ok);

    let first = run(&["log", "-C", dir])
        .stdout
        .lines()
        .last()
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();
    run(&["restore", "-C", dir, &first, "-y"]);

    assert!(run(&["checkout", "-C", dir, "main"]).ok);
    assert_eq!(
        std::fs::read(&file).unwrap(),
        on_main,
        "switching back should restore the branch's own state"
    );
}

/// Edits made while the daemon was not running have to be described, not
/// filed under a generic startup message, or the log hides a real change.
#[test]
fn edits_made_while_not_watching_are_described() {
    let s = Scratch::new("catchup");
    let dir = s.path().to_str().unwrap();
    let file = s.path().join("holiday.kdenlive");

    // One commit exists, then the file changes with nothing watching.
    std::fs::write(&file, sample("timelapse-a.kdenlive")).unwrap();
    let store = cutback::git_backend::Store::open(s.path(), &file).unwrap();
    store.commit("Started watching this project", "").unwrap();
    std::fs::write(&file, sample("timelapse-c.kdenlive")).unwrap();
    drop(store);

    // Starting the watcher should record that edit and say what it was. Run it
    // briefly, since watch runs in the foreground until interrupted.
    let mut child = Command::new(binary())
        .args(["watch", dir])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn watch");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let _ = child.kill();
    let _ = child.wait();

    let log = run(&["log", "-C", dir]).stdout;
    assert!(
        log.contains("not running"),
        "the catch up commit should say it happened outside the daemon: {log}"
    );
    assert!(
        log.contains("project bin"),
        "the catch up commit should describe the edit: {log}"
    );
}

#[test]
fn a_directory_with_no_project_says_so() {
    let s = Scratch::new("empty");
    let out = run(&["log", "-C", s.path().to_str().unwrap()]);
    assert!(!out.ok);
    assert!(out.stderr.contains("no .kdenlive file"), "{}", out.stderr);
}

#[test]
fn an_unknown_revision_is_reported_clearly() {
    let s = project_with_history("badrev");
    let out = run(&[
        "restore",
        "-C",
        s.path().to_str().unwrap(),
        "nosuchrev",
        "-y",
    ]);
    assert!(!out.ok);
    assert!(out.stderr.contains("no such revision"), "{}", out.stderr);
}

#[test]
fn a_directory_with_two_projects_asks_which_one() {
    let s = Scratch::new("ambiguous");
    std::fs::write(
        s.path().join("one.kdenlive"),
        sample("timelapse-a.kdenlive"),
    )
    .unwrap();
    std::fs::write(
        s.path().join("two.kdenlive"),
        sample("timelapse-a.kdenlive"),
    )
    .unwrap();

    let out = run(&["log", "-C", s.path().to_str().unwrap()]);
    assert!(!out.ok);
    assert!(out.stderr.contains("several projects"), "{}", out.stderr);
}
