//! Git storage, through libgit2.
//!
//! Nothing here knows about Kdenlive or XML. It tracks one file inside a
//! repository and is deliberately kept that way, so the interesting logic
//! stays in the parser and differ where it can be tested on its own.
//!
//! The repository lives in a `.cutback` directory beside the project rather
//! than in `.git`, so that a project folder that is already a git repository
//! keeps working normally and the editor's own version control is untouched.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use git2::{
    build::CheckoutBuilder, Commit, IndexEntry, IndexTime, ObjectType, Oid, Repository, Signature,
    Sort,
};

pub const STORE_DIR: &str = ".cutback";

pub struct Store {
    repo: Repository,
    /// Path of the tracked file, relative to the work tree.
    tracked: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub short_id: String,
    pub subject: String,
    pub body: String,
    pub seconds_since_epoch: i64,
}

impl Store {
    /// Opens the store for a project directory, creating it on first use.
    ///
    /// `project_file` must sit inside `dir`. The repository work tree is `dir`
    /// itself, so the tracked file is committed in place and a restore writes
    /// back to exactly where Kdenlive expects it.
    pub fn open(dir: &Path, project_file: &Path) -> Result<Self> {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("no such directory: {}", dir.display()))?;
        let tracked = project_file
            .file_name()
            .map(PathBuf::from)
            .context("project path has no file name")?;

        let git_dir = dir.join(STORE_DIR);
        let repo = if git_dir.exists() {
            Repository::open(&git_dir)
                .with_context(|| format!("opening {}", git_dir.display()))?
        } else {
            let repo = Repository::init_bare(&git_dir)
                .with_context(|| format!("creating {}", git_dir.display()))?;
            repo.set_workdir(&dir, false)?;
            // A bare init leaves core.bare set, which blocks checkout later.
            repo.config()?.set_bool("core.bare", false)?;
            repo
        };
        repo.set_workdir(&dir, false)?;

        Ok(Store { repo, tracked })
    }

    pub fn work_dir(&self) -> Result<&Path> {
        self.repo.workdir().context("store has no work tree")
    }

    pub fn tracked_path(&self) -> Result<PathBuf> {
        Ok(self.work_dir()?.join(&self.tracked))
    }

    /// Commits the current contents of the tracked file.
    ///
    /// Returns None when the file is byte for byte identical to what the last
    /// commit holds. Kdenlive rewrites the whole document on every save, so
    /// this is the guard against empty commits when a save changed nothing.
    pub fn commit(&self, subject: &str, body: &str) -> Result<Option<Oid>> {
        let path = self.tracked_path()?;
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading {}", path.display()))?;

        let blob = self.repo.blob(&bytes)?;
        if let Some(parent) = self.head_commit()? {
            if let Ok(existing) = parent.tree()?.get_path(&self.tracked) {
                if existing.id() == blob {
                    return Ok(None);
                }
            }
        }

        // Build the tree from the blob directly instead of staging from disk,
        // so nothing between here and the object store can rewrite the bytes.
        let mut builder = self.repo.treebuilder(None)?;
        let name = self.tracked.to_str().context("project file name is not UTF-8")?;
        builder.insert(name, blob, 0o100_644)?;
        let tree = self.repo.find_tree(builder.write()?)?;

        let who = self.signature()?;
        let message = if body.is_empty() {
            subject.to_string()
        } else {
            format!("{subject}\n\n{body}")
        };

        let parents: Vec<Commit> = self.head_commit()?.into_iter().collect();
        let parent_refs: Vec<&Commit> = parents.iter().collect();

        let oid = self.repo.commit(
            Some("HEAD"),
            &who,
            &who,
            &message,
            &tree,
            &parent_refs,
        )?;

        // Keep the index in step with the new commit, otherwise libgit2 sees
        // the work tree as dirty on the next checkout and refuses it.
        self.sync_index(blob, &bytes)?;
        Ok(Some(oid))
    }

    /// Points the index at the committed blob without touching the file.
    fn sync_index(&self, blob: Oid, bytes: &[u8]) -> Result<()> {
        let mut index = self.repo.index()?;
        let name = self.tracked.to_str().context("project file name is not UTF-8")?;
        index.add(&IndexEntry {
            ctime: IndexTime::new(0, 0),
            mtime: IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: 0o100_644,
            uid: 0,
            gid: 0,
            file_size: bytes.len() as u32,
            id: blob,
            flags: 0,
            flags_extended: 0,
            path: name.into(),
        })?;
        index.write()?;
        Ok(())
    }

    pub fn log(&self, limit: Option<usize>) -> Result<Vec<Entry>> {
        if self.head_commit()?.is_none() {
            return Ok(Vec::new());
        }
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        // Sort by ancestry, not timestamp. Commit times have one second
        // granularity, and saves made within the same second are exactly the
        // workload here, so ordering by time would shuffle rapid saves.
        walk.set_sorting(Sort::TOPOLOGICAL)?;

        let mut out = Vec::new();
        for oid in walk {
            let commit = self.repo.find_commit(oid?)?;
            out.push(entry_from(&commit));
            if limit.is_some_and(|n| out.len() >= n) {
                break;
            }
        }
        Ok(out)
    }

    /// Reads the tracked file's contents at a revision, without touching disk.
    pub fn file_at(&self, rev: &str) -> Result<Vec<u8>> {
        let commit = self.resolve(rev)?;
        let entry = commit
            .tree()?
            .get_path(&self.tracked)
            .with_context(|| format!("{rev} does not contain {}", self.tracked.display()))?;
        let blob = self.repo.find_blob(entry.id())?;
        Ok(blob.content().to_vec())
    }

    /// Writes the tracked file back to its state at `rev`.
    ///
    /// The bytes come straight from the stored blob, so the file that lands on
    /// disk is the one Kdenlive originally wrote, with no reformatting.
    pub fn restore(&self, rev: &str) -> Result<()> {
        let bytes = self.file_at(rev)?;
        let path = self.tracked_path()?;
        write_atomically(&path, &bytes)
            .with_context(|| format!("restoring {}", path.display()))?;

        let blob = self.repo.blob(&bytes)?;
        self.sync_index(blob, &bytes)?;
        Ok(())
    }

    pub fn resolve(&self, rev: &str) -> Result<Commit<'_>> {
        let object = self
            .repo
            .revparse_single(rev)
            .with_context(|| format!("no such revision: {rev}"))?;
        let commit = object
            .peel(ObjectType::Commit)
            .with_context(|| format!("{rev} is not a commit"))?
            .into_commit()
            .map_err(|_| anyhow::anyhow!("{rev} is not a commit"))?;
        Ok(commit)
    }

    pub fn branch(&self, name: &str) -> Result<()> {
        let head = self
            .head_commit()?
            .context("cannot branch before the first save has been recorded")?;
        self.repo.branch(name, &head, false)?;
        Ok(())
    }

    pub fn checkout(&self, name: &str) -> Result<()> {
        let reference = self
            .repo
            .find_branch(name, git2::BranchType::Local)
            .with_context(|| format!("no such branch: {name}"))?
            .into_reference();
        let refname = reference.name().context("branch name is not UTF-8")?.to_string();

        let tree = reference.peel(ObjectType::Tree)?;
        // Force is deliberate. The point of a checkout here is to make the
        // project file match the branch, and the daemon has already committed
        // whatever was on disk before this ran.
        self.repo
            .checkout_tree(&tree, Some(CheckoutBuilder::new().force()))?;
        self.repo.set_head(&refname)?;
        Ok(())
    }

    pub fn current_branch(&self) -> Result<String> {
        let head = match self.repo.head() {
            Ok(head) => head,
            // A fresh repository has no HEAD commit yet.
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => return Ok("main".to_string()),
            Err(e) => return Err(e.into()),
        };
        Ok(head.shorthand().unwrap_or("HEAD").to_string())
    }

    pub fn branches(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for branch in self.repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = branch?;
            if let Some(name) = branch.name()? {
                out.push(name.to_string());
            }
        }
        Ok(out)
    }

    fn head_commit(&self) -> Result<Option<Commit<'_>>> {
        match self.repo.head() {
            Ok(head) => Ok(Some(head.peel_to_commit()?)),
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Commits are attributed to cutback rather than the user's git identity,
    /// since these are automatic saves and not authored work. This also keeps
    /// the daemon working when git has no user configured.
    fn signature(&self) -> Result<Signature<'static>> {
        Ok(Signature::now("cutback", "cutback@localhost")?)
    }
}

fn entry_from(commit: &Commit) -> Entry {
    let message = commit.message().unwrap_or_default();
    let (subject, body) = match message.split_once("\n\n") {
        Some((s, b)) => (s.trim(), b.trim()),
        None => (message.trim(), ""),
    };
    Entry {
        id: commit.id().to_string(),
        short_id: commit.id().to_string()[..7].to_string(),
        subject: subject.to_string(),
        body: body.to_string(),
        seconds_since_epoch: commit.time().seconds(),
    }
}

/// Writes through a temporary file in the same directory, then renames.
///
/// A rename within one filesystem is atomic, so a reader either sees the old
/// file or the new one. Writing in place would leave a half written project
/// file if the process died partway, which is the failure this tool exists to
/// prevent.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().context("path has no parent directory")?;
    let temp = dir.join(format!(
        ".{}.cutback-tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("project")
    ));

    std::fs::write(&temp, bytes)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        bail!(e);
    }
    Ok(())
}
