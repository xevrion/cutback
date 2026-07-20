//! Command definitions and the code that wires the modules together.
//!
//! This layer stays thin on purpose. Anything worth testing lives in the
//! parser, differ or store, where it can be tested without a terminal.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::differ::diff;
use crate::git_backend::Store;
use crate::model::Project;
use crate::render::{display, render, summarize};
use crate::term;
use crate::watcher::Watcher;
use crate::xml_parser;

#[derive(Parser)]
#[command(
    name = "cutback",
    version,
    about = "Version control for Kdenlive projects, in plain English",
    long_about = "cutback watches a Kdenlive project and commits every save to a local git \n\
                  repository, then describes what changed in sentences instead of XML.\n\n\
                  Start with 'cutback watch' in one terminal and keep editing as usual.",
    after_help = "Run 'cutback <command> --help' for details on a command.\n\
                  Full documentation: man cutback",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Watch a project and commit every save
    #[command(
        long_about = "Watches the project for saves and commits each one automatically.\n\n\
                            Runs in the foreground. Leave it running while you edit, and stop it \n\
                            with Ctrl-C. Saves made while it is not running are picked up the \n\
                            next time it starts."
    )]
    Watch {
        /// Project directory, or the .kdenlive file itself
        #[arg(value_name = "PATH", default_value = ".")]
        path: PathBuf,
    },

    /// Show the history in plain English
    Log {
        #[command(flatten)]
        project: ProjectArg,

        /// Limit the number of entries shown
        #[arg(short = 'n', long, value_name = "COUNT")]
        limit: Option<usize>,

        /// Show the full change list for each entry, not just the summary
        #[arg(long)]
        full: bool,
    },

    /// Describe what changed between two points in history
    #[command(long_about = "Describes what changed between two revisions.\n\n\
                            With no arguments, compares the two most recent saves. A revision is \n\
                            a commit id from 'cutback log', or anything git understands such as \n\
                            HEAD~3 or a branch name.")]
    Diff {
        #[command(flatten)]
        project: ProjectArg,

        /// Revision to compare from
        #[arg(value_name = "REV1")]
        rev1: Option<String>,

        /// Revision to compare to
        #[arg(value_name = "REV2")]
        rev2: Option<String>,
    },

    /// Restore the project to an earlier revision
    #[command(
        long_about = "Restores the project file to its exact state at a revision.\n\n\
                            The restored file is byte for byte what Kdenlive wrote at that point. \n\
                            Close the project in Kdenlive first, otherwise it will overwrite the \n\
                            restored file on its next save."
    )]
    Restore {
        #[command(flatten)]
        project: ProjectArg,

        /// Revision to restore, from 'cutback log'
        #[arg(value_name = "REV")]
        rev: String,

        /// Restore without asking for confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Create a branch from the current state
    #[command(
        long_about = "Creates a branch at the current state, for trying an alternate cut.\n\n\
                            Creating a branch does not switch to it. Use 'cutback checkout' for \n\
                            that."
    )]
    Branch {
        #[command(flatten)]
        project: ProjectArg,

        /// Name for the new branch
        #[arg(value_name = "NAME")]
        name: Option<String>,
    },

    /// Switch to a branch, restoring the project to its state
    Checkout {
        #[command(flatten)]
        project: ProjectArg,

        /// Branch to switch to
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Print a shell completion script
    #[command(long_about = "Prints a completion script for the given shell.\n\n\
                            bash:  cutback completions bash > \
                            ~/.local/share/bash-completion/completions/cutback\n\
                            zsh:   cutback completions zsh > ~/.zfunc/_cutback\n\
                            fish:  cutback completions fish > \
                            ~/.config/fish/completions/cutback.fish")]
    Completions {
        /// Shell to generate for
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
}

#[derive(Args)]
struct ProjectArg {
    /// Project directory, or the .kdenlive file itself
    #[arg(
        short = 'C',
        long = "project",
        value_name = "PATH",
        default_value = ".",
        global = true
    )]
    path: PathBuf,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Watch { path } => watch(&path),
        Command::Log {
            project,
            limit,
            full,
        } => log(&project.path, limit, full),
        Command::Diff {
            project,
            rev1,
            rev2,
        } => show_diff(&project.path, rev1, rev2),
        Command::Restore { project, rev, yes } => restore(&project.path, &rev, yes),
        Command::Branch { project, name } => branch(&project.path, name),
        Command::Checkout { project, name } => checkout(&project.path, &name),
        Command::Completions { shell } => {
            use clap::CommandFactory;
            let mut command = Cli::command();
            clap_complete::generate(shell, &mut command, "cutback", &mut std::io::stdout());
            Ok(())
        }
    }
}

/// Resolves a path that may be a directory or the project file itself.
fn locate(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let path = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };

    if path.is_file() {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let dir = if dir.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            dir
        };
        return Ok((dir, path.to_path_buf()));
    }

    if !path.is_dir() {
        bail!("no such file or directory: {}", path.display());
    }

    let mut found: Vec<PathBuf> = std::fs::read_dir(path)
        .with_context(|| format!("reading {}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "kdenlive"))
        .collect();
    found.sort();

    match found.len() {
        0 => bail!(
            "no .kdenlive file in {}. Pass the project file with -C <path>",
            path.display()
        ),
        1 => Ok((path.to_path_buf(), found.remove(0))),
        _ => {
            let names: Vec<String> = found
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            bail!(
                "{} holds several projects ({}). Pass one with -C <path>",
                path.display(),
                names.join(", ")
            )
        }
    }
}

fn open(path: &Path) -> Result<(Store, PathBuf)> {
    let (dir, file) = locate(path)?;
    let store = Store::open(&dir, &file)?;
    Ok((store, file))
}

fn watch(path: &Path) -> Result<()> {
    let (dir, file) = locate(path)?;
    let store = Store::open(&dir, &file)?;

    // Parse once before watching. Committing an unparseable file would put
    // history in a state we cannot describe later.
    let mut previous = xml_parser::parse_file(&file)?;
    let fps = previous.profile.fps();

    // Record the state at startup so that edits made while the daemon was not
    // running still land in history. Describe them properly where we can:
    // the file on disk may have moved on since the last commit, and labelling
    // that "started watching" would hide a real edit in the log.
    let catch_up = match store.log(Some(1))?.first() {
        Some(last) => parse_revision(&store, &last.id)
            .ok()
            .map(|committed| diff(&committed, &previous)),
        None => None,
    };

    let (subject, body) = match &catch_up {
        Some(changes) if !changes.is_empty() => {
            let lines = render(changes, fps);
            (
                format!(
                    "{} (saved while cutback was not running)",
                    summarize(changes, fps)
                ),
                lines.join("\n"),
            )
        }
        _ => ("Started watching this project".to_string(), String::new()),
    };

    let recorded = store.commit(&subject, &body)?.is_some();

    let branch = store.current_branch()?;
    let saves = store.log(None)?.len();
    println!(
        "{} {}  {}",
        term::bold("watching"),
        term::subject(&display_name(&file)),
        term::dim(&format!(
            "on {branch}, {saves} save{} recorded",
            if saves == 1 { "" } else { "s" }
        ))
    );

    if recorded {
        if let Some(changes) = &catch_up {
            if !changes.is_empty() {
                println!(
                    "{}",
                    term::dim("caught up on edits made while cutback was not running:")
                );
                for line in display(changes, fps) {
                    println!("       {} {}", term::dim("·"), line);
                }
            }
        }
    }

    println!(
        "{}",
        term::dim("save in Kdenlive and the change appears here. Ctrl-C to stop")
    );
    println!();

    let watcher = Watcher::new(&file)?;
    loop {
        if !watcher.wait_for_save(Duration::from_secs(60))? {
            continue;
        }

        let current = match xml_parser::parse_file(&file) {
            Ok(project) => project,
            // A save we cannot parse is not committed, since we could not
            // describe it. Report it and keep watching rather than exiting,
            // because the next save is usually fine.
            Err(e) => {
                eprintln!("cutback: skipped a save, {e}");
                continue;
            }
        };

        let changes = diff(&previous, &current);
        if changes.is_empty() {
            // Kdenlive rewrites the whole file on every save, so a save with
            // no edits is normal and should not make a commit.
            continue;
        }

        // Plain text goes into the commit, coloured text goes to the screen.
        // Escape codes stored in a message would be in history for good.
        let subject = summarize(&changes, fps);
        let body = render(&changes, fps).join("\n");
        let shown = display(&changes, fps);

        match store.commit(&subject, &body)? {
            Some(oid) => {
                println!(
                    "{} {}  {}",
                    term::dim(&clock_time()),
                    term::commit_id(&oid.to_string()[..7]),
                    summarize_headline(shown.len()),
                );
                for line in &shown {
                    println!("       {} {}", term::dim("·"), line);
                }
            }
            None => continue,
        }
        previous = current;
    }
}

fn log(path: &Path, limit: Option<usize>, full: bool) -> Result<()> {
    let (store, _) = open(path)?;
    let entries = store.log(limit)?;

    if entries.is_empty() {
        println!("no history yet. Run 'cutback watch' and save in Kdenlive");
        return Ok(());
    }

    let branch = store.current_branch()?;
    let head = entries.first().map(|e| e.id.clone()).unwrap_or_default();

    for entry in &entries {
        let marker = if entry.id == head {
            term::bold("*")
        } else {
            term::dim("*")
        };
        let when = relative_time(entry.seconds_since_epoch);
        let here = if entry.id == head {
            format!(" {}", term::dim(&format!("(now, on {branch})")))
        } else {
            String::new()
        };

        println!(
            "{marker} {}  {}  {}{here}",
            term::commit_id(&entry.short_id),
            term::dim(&term::pad(&when, 14)),
            entry.subject,
        );

        if full && !entry.body.is_empty() {
            for line in entry.body.lines() {
                println!("{}   {}", term::dim("│"), line);
            }
            if entry.id != entries[entries.len() - 1].id {
                println!("{}", term::dim("│"));
            }
        }
    }

    // Hints are for a person reading the output, and would be noise or worse
    // to anything parsing it, so they stop at the terminal.
    if !full && entries.len() > 1 && term::is_tty() {
        println!();
        println!(
            "{}",
            term::dim(
                "see every change with 'cutback log --full', or one save with 'cutback diff <id>'"
            )
        );
    }
    Ok(())
}

fn show_diff(path: &Path, rev1: Option<String>, rev2: Option<String>) -> Result<()> {
    let (store, _) = open(path)?;

    // With no revisions given, compare the two most recent saves.
    let (from, to) = match (rev1, rev2) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (format!("{a}~1"), a),
        (None, _) => {
            let entries = store.log(Some(2))?;
            match entries.as_slice() {
                [] => bail!("no history yet. Run 'cutback watch' and save in Kdenlive"),
                [_only] => bail!("only one save recorded so far, nothing to compare it against"),
                [newest, older, ..] => (older.id.clone(), newest.id.clone()),
            }
        }
    };

    let before = parse_revision(&store, &from)?;
    let after = parse_revision(&store, &to)?;

    let changes = diff(&before, &after);
    let lines = display(&changes, after.profile.fps());

    // Say which two saves this is, since with no arguments the user did not
    // pick them and otherwise has no idea what is being compared.
    let (from_commit, to_commit) = (store.resolve(&from)?, store.resolve(&to)?);
    let describe = |c: &git2::Commit| {
        format!(
            "{} {}",
            term::commit_id(&c.id().to_string()[..7]),
            term::dim(c.summary().unwrap_or("(no description)"))
        )
    };
    println!("{} {}", term::dim("from"), describe(&from_commit));
    println!("{}   {}", term::dim("to"), describe(&to_commit));
    println!();

    if lines.is_empty() {
        println!("{}", term::dim("no changes between these two saves"));
        return Ok(());
    }
    for line in lines {
        println!("  {}", line);
    }
    Ok(())
}

fn parse_revision(store: &Store, rev: &str) -> Result<Project> {
    let bytes = store.file_at(rev)?;
    let text = String::from_utf8(bytes)
        .with_context(|| format!("the project stored at {rev} is not valid UTF-8"))?;
    xml_parser::parse_str(&text).with_context(|| format!("reading the project at {rev}"))
}

fn restore(path: &Path, rev: &str, assume_yes: bool) -> Result<()> {
    let (store, file) = open(path)?;
    let commit = store.resolve(rev)?;
    let subject = commit.summary().unwrap_or("(no description)").to_string();

    if !assume_yes {
        println!(
            "This overwrites {} with its state at {}:",
            display_name(&file),
            &commit.id().to_string()[..7]
        );
        println!("  {subject}");
        println!();
        println!("Close the project in Kdenlive first, or it will save over the restored file.");
        if !confirm("Restore?")? {
            println!("nothing changed");
            return Ok(());
        }
    }

    // Keep whatever is on disk now, so a restore is never a one way door.
    let saved_first = store.commit("Saved before a restore", "")?.is_some();
    if saved_first {
        println!("{}", term::dim("recorded the current state first"));
    }

    store.restore(rev)?;
    let short = &commit.id().to_string()[..7];
    println!(
        "{} {} is back at {} {}",
        term::green("restored"),
        term::subject(&display_name(&file)),
        term::commit_id(short),
        term::dim(commit.summary().unwrap_or("")),
    );
    if saved_first {
        println!(
            "{}",
            term::dim("the state you had before this is the newest entry in 'cutback log'")
        );
    }
    Ok(())
}

fn branch(path: &Path, name: Option<String>) -> Result<()> {
    let (store, _) = open(path)?;

    let Some(name) = name else {
        let current = store.current_branch()?;
        let branches = store.branches()?;
        for branch in &branches {
            if *branch == current {
                println!("{} {}", term::green("*"), term::bold(branch));
            } else {
                println!("  {branch}");
            }
        }
        if branches.len() == 1 {
            println!();
            println!(
                "{}",
                term::dim("start an alternate cut with 'cutback branch <name>'")
            );
        }
        return Ok(());
    };

    store.branch(&name)?;
    println!("{} branch {}", term::green("created"), term::subject(&name));
    println!(
        "{}",
        term::dim(&format!("switch to it with 'cutback checkout {name}'"))
    );
    Ok(())
}

fn checkout(path: &Path, name: &str) -> Result<()> {
    let (store, file) = open(path)?;
    let previous = store.current_branch()?;

    // Committing first means an uncommitted edit is not lost by the switch.
    if store
        .commit("Saved before switching branches", "")?
        .is_some()
    {
        println!(
            "{}",
            term::dim(&format!("recorded the current state on {previous} first"))
        );
    }

    store.checkout(name)?;
    println!(
        "{} {}, {} now holds that branch's state",
        term::green("switched to"),
        term::subject(name),
        term::subject(&display_name(&file)),
    );
    println!("{}", term::dim("reopen the project in Kdenlive to see it"));
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    use std::io::{BufRead, Write};

    print!("{question} [y/N] ");
    std::io::stdout().flush()?;

    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn display_name(file: &Path) -> String {
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.display().to_string())
}

/// Headline for one save in the watch log.
///
/// Every change is printed underneath, so repeating the first one in the
/// headline is noise. When there are several, say how many; when there is
/// one, the detail line below already says it and the headline can be short.
fn summarize_headline(count: usize) -> String {
    match count {
        1 => "1 change".to_string(),
        n => format!("{n} changes"),
    }
}

/// Wall clock time for the watch log, where entries arrive as you work and
/// "3 minutes ago" would be wrong by the time you read it.
fn clock_time() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Local time without pulling in a date library. The offset comes from the
    // difference the OS reports for this timestamp.
    let secs_today = (now % 86_400) as i64 + local_offset_seconds();
    let secs_today = secs_today.rem_euclid(86_400);
    format!("{:02}:{:02}", secs_today / 3600, (secs_today % 3600) / 60)
}

/// Seconds east of UTC, read once from the TZ database via libc's localtime.
fn local_offset_seconds() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        // date is present on every Linux and avoids a chrono dependency for
        // one number that cannot change while the process runs.
        std::process::Command::new("date")
            .arg("+%z")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                let s = s.trim();
                let sign = if s.starts_with('-') { -1 } else { 1 };
                let digits = s.trim_start_matches(['+', '-']);
                let hours: i64 = digits.get(0..2)?.parse().ok()?;
                let minutes: i64 = digits.get(2..4)?.parse().ok()?;
                Some(sign * (hours * 3600 + minutes * 60))
            })
            .unwrap_or(0)
    })
}

/// Commit times read better as "20 minutes ago" than as a date when you are
/// looking for the save you made a moment ago.
fn relative_time(seconds_since_epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(seconds_since_epoch);

    let elapsed = now.saturating_sub(seconds_since_epoch).max(0);
    let (value, unit) = match elapsed {
        s if s < 60 => return "just now".to_string(),
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 2_592_000 => (s / 86_400, "day"),
        s => (s / 2_592_000, "month"),
    };
    let plural = if value == 1 { "" } else { "s" };
    format!("{value} {unit}{plural} ago")
}
