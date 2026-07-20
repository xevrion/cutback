//! Watches a project file and reports completed saves.
//!
//! The signal we key on is what tells us Kdenlive finished writing, not a
//! timer. Kdenlive saves through QSaveFile, which writes a temporary file in
//! the same directory and renames it over the project. Two consequences:
//!
//! - The project file itself is never written in place, so inotify's
//!   IN_CLOSE_WRITE never fires for its path. Watching only for close-write
//!   would miss every save. IN_MOVED_TO, the rename landing, is the real
//!   completion signal.
//! - Because the rename is atomic, a reader either sees the whole old file or
//!   the whole new one. There is no window where a partial document is
//!   visible under the project's name.
//!
//! Save As and other tools may still write in place, so close-write is treated
//! as a completed save too. Both events mean the writer is done, which is why
//! this is correct where a fixed debounce delay would only be a guess about
//! how long writing takes.
//!
//! The directory is watched rather than the file, since watching an inode that
//! gets replaced by a rename would leave us following the old file.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::event::{CreateKind, EventKind, ModifyKind, RenameMode};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// A single save can produce more than one qualifying event, for instance a
/// rename landing followed by a metadata update. Collapsing events this close
/// together keeps one save to one commit.
///
/// This is not how we detect that a write finished, only how we avoid counting
/// one finished write twice.
const COALESCE_WINDOW: Duration = Duration::from_millis(300);

pub struct Watcher {
    /// Held because dropping it stops the watch.
    _inner: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    file: PathBuf,
}

impl Watcher {
    pub fn new(project_file: &Path) -> Result<Self> {
        let file = project_file.to_path_buf();
        let dir = project_file
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let (tx, events) = mpsc::channel();
        let mut inner = notify::recommended_watcher(move |res| {
            // A send failure means the receiver is gone and the daemon is
            // shutting down, so there is nothing useful to do about it.
            let _ = tx.send(res);
        })?;
        inner
            .watch(&dir, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", dir.display()))?;

        Ok(Watcher {
            _inner: inner,
            events,
            file,
        })
    }

    /// Blocks until the project file has been completely written, or until
    /// `timeout` passes with no save.
    ///
    /// Returns Ok(false) on timeout, which lets the caller check whether it
    /// has been asked to shut down.
    pub fn wait_for_save(&self, timeout: Duration) -> Result<bool> {
        let deadline = Instant::now() + timeout;

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(false);
            }

            match self.events.recv_timeout(remaining) {
                Ok(Ok(event)) if self.is_completed_save(&event) => {
                    self.drain_coalescing_window();
                    return Ok(true);
                }
                Ok(Ok(_)) => continue,
                // Losing events (queue overflow) is worth reporting, but it is
                // not fatal: the next save will be picked up normally.
                Ok(Err(e)) => {
                    eprintln!("cutback: watch error: {e}");
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => return Ok(false),
                Err(RecvTimeoutError::Disconnected) => {
                    anyhow::bail!("file watcher stopped unexpectedly")
                }
            }
        }
    }

    fn is_completed_save(&self, event: &notify::Event) -> bool {
        if !event.paths.iter().any(|p| self.is_project_file(p)) {
            return false;
        }
        matches!(
            event.kind,
            // The rename landing, which is how Kdenlive finishes a save.
            EventKind::Modify(ModifyKind::Name(RenameMode::To))
                | EventKind::Create(CreateKind::File)
                // An in place write that has been closed, for tools that do not
                // save atomically.
                | EventKind::Access(notify::event::AccessKind::Close(
                    notify::event::AccessMode::Write
                ))
        )
    }

    fn is_project_file(&self, path: &Path) -> bool {
        // Compare by name as well as full path, since notify reports paths as
        // the OS gives them and the watched directory may be a symlink.
        path == self.file || path.file_name() == self.file.file_name()
    }

    /// Swallows any further events that belong to the same save.
    fn drain_coalescing_window(&self) {
        let until = Instant::now() + COALESCE_WINDOW;
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            if self.events.recv_timeout(remaining).is_err() {
                return;
            }
        }
    }
}
