//! Watcher tests.
//!
//! Saves here are performed the way Kdenlive performs them, writing a
//! temporary file in the same directory and renaming it over the project.
//! That is what QSaveFile does, so testing any other way would prove nothing
//! about the real case.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cutback::watcher::Watcher;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("cutback-watch-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("scratch dir");
        Scratch(base)
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

/// Writes the way Kdenlive does: temp file in the same directory, fsync, then
/// an atomic rename over the target.
fn save_like_kdenlive(path: &Path, contents: &str) {
    let temp = path.with_extension("kdenlive.tmp");
    {
        let mut f = std::fs::File::create(&temp).expect("temp file");
        f.write_all(contents.as_bytes()).expect("write");
        f.sync_all().expect("fsync");
    }
    std::fs::rename(&temp, path).expect("rename into place");
}

#[test]
fn detects_an_atomic_save() {
    let s = Scratch::new("atomic");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "before").unwrap();

    let watcher = Watcher::new(&project).unwrap();
    let saver = std::thread::spawn({
        let project = project.clone();
        move || {
            std::thread::sleep(Duration::from_millis(150));
            save_like_kdenlive(&project, "after");
        }
    });

    assert!(
        watcher.wait_for_save(Duration::from_secs(5)).unwrap(),
        "a rename into place is a completed save"
    );
    saver.join().unwrap();
}

/// The file the watcher sees must always be complete. With an atomic rename
/// there is no window where a half written document is visible, which is the
/// property that keeps the daemon from committing a truncated project.
#[test]
fn the_project_file_is_never_seen_half_written() {
    let s = Scratch::new("complete");
    let project = s.file("p.kdenlive");
    let full = "x".repeat(200_000);
    std::fs::write(&project, &full).unwrap();

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader = std::thread::spawn({
        let project = project.clone();
        let stop = stop.clone();
        move || {
            let mut seen_short = 0;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok(text) = std::fs::read_to_string(&project) {
                    // Every read must land on one whole version or the other.
                    if text.len() != 200_000 && text.len() != 300_000 {
                        seen_short += 1;
                    }
                }
            }
            seen_short
        }
    });

    for _ in 0..20 {
        save_like_kdenlive(&project, &"y".repeat(300_000));
        save_like_kdenlive(&project, &"x".repeat(200_000));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);

    assert_eq!(
        reader.join().unwrap(),
        0,
        "a reader must never observe a partially written project file"
    );
}

/// Rapid saves must not be lost. Kdenlive users hit ctrl+s repeatedly, and a
/// missed save means an edit that never made it into history.
#[test]
fn rapid_saves_are_all_detected() {
    let s = Scratch::new("rapid");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "start").unwrap();

    let watcher = Watcher::new(&project).unwrap();

    // Space these beyond the coalescing window so each is a distinct save.
    let saver = std::thread::spawn({
        let project = project.clone();
        move || {
            for i in 0..3 {
                std::thread::sleep(Duration::from_millis(500));
                save_like_kdenlive(&project, &format!("save {i}"));
            }
        }
    });

    let mut detected = 0;
    for _ in 0..3 {
        if watcher.wait_for_save(Duration::from_secs(5)).unwrap() {
            detected += 1;
        }
    }
    saver.join().unwrap();
    assert_eq!(detected, 3, "each distinct save should be reported once");
}

/// One save must not report twice, or the daemon would make empty commits.
#[test]
fn one_save_is_reported_once() {
    let s = Scratch::new("once");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "before").unwrap();

    let watcher = Watcher::new(&project).unwrap();
    save_like_kdenlive(&project, "after");

    assert!(watcher.wait_for_save(Duration::from_secs(5)).unwrap());
    // Nothing else happened, so the next wait should time out rather than
    // report the same save again.
    assert!(
        !watcher.wait_for_save(Duration::from_millis(800)).unwrap(),
        "the same save must not be reported twice"
    );
}

#[test]
fn ignores_other_files_in_the_directory() {
    let s = Scratch::new("others");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "x").unwrap();
    let watcher = Watcher::new(&project).unwrap();

    // Kdenlive drops audio thumbnails, backups and lock files beside the
    // project. None of them is a save.
    for noise in ["p.kdenlive.audio", "other.kdenlive", ".p.kdenlive.lock"] {
        std::fs::write(s.file(noise), "noise").unwrap();
    }

    assert!(
        !watcher.wait_for_save(Duration::from_millis(800)).unwrap(),
        "writes to other files must not count as a save"
    );
}

#[test]
fn times_out_when_nothing_happens() {
    let s = Scratch::new("idle");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "x").unwrap();

    let watcher = Watcher::new(&project).unwrap();
    let start = std::time::Instant::now();
    assert!(!watcher.wait_for_save(Duration::from_millis(400)).unwrap());
    assert!(start.elapsed() >= Duration::from_millis(350));
}

#[test]
fn detects_an_in_place_write() {
    let s = Scratch::new("inplace");
    let project = s.file("p.kdenlive");
    std::fs::write(&project, "before").unwrap();

    let watcher = Watcher::new(&project).unwrap();
    let saver = std::thread::spawn({
        let project = project.clone();
        move || {
            std::thread::sleep(Duration::from_millis(150));
            // Not every tool saves atomically, so a closed in place write
            // counts as a completed save too.
            let mut f = std::fs::File::create(&project).unwrap();
            f.write_all(b"after").unwrap();
        }
    });

    assert!(watcher.wait_for_save(Duration::from_secs(5)).unwrap());
    saver.join().unwrap();
}
