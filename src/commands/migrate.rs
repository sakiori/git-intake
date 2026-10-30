//! `migrate`: carry an update in progress over from the previous state
//! format to the current one, in place.
//!
//! Format 3 kept the update worktree in the git directory, where compilers,
//! bundlers and watchers ignore every path, so checks run there may not see
//! the tree as it is. `migrate` moves it to where `start` places it now with
//! `git worktree move`, which keeps the branch, index, merge in progress and
//! files, and only then writes the state in the current format. A run
//! interrupted between the two finds the worktree at its new place and
//! writes the state.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::update::next_step;
use crate::git::{Git, abbrev};
use crate::repo::{MainWorktree, Repo, canonical};
use crate::state::{self, Stored, UPDATE_BRANCH};

pub fn run(repo: &Repo) -> Result<Exit> {
    let state = match state::load_stored(&repo.paths.state)? {
        None => bail!("no update is in progress; there is nothing to migrate"),
        Some(Stored::Current(_)) => bail!(
            "the update in progress already keeps its state in format {}; there is nothing to migrate",
            state::current_format()
        ),
        Some(Stored::Previous(state)) => *state,
    };
    let old = state.worktree.clone();
    let mut moved_from_here = None;
    let new = if stays(repo, &old) {
        println!("the update worktree stays at {}", old.display());
        old.clone()
    } else {
        let new = repo.place_update_worktree()?;
        let listed = repo.worktrees()?;
        let registered = |path: &Path| {
            let path = canonical(path);
            listed
                .iter()
                .find(|w| w.path == path)
                .map(|w| w.branch.as_deref() == Some(UPDATE_BRANCH))
        };
        match (registered(&old), registered(&new)) {
            // An earlier `migrate` moved the worktree but did not write the
            // state.
            (None, Some(true)) => {
                println!("the update worktree is already at {}", new.display());
            }
            // Moved without git, by hand or by a `git worktree move` stopped
            // before it updated its records: git still lists the old path.
            (Some(true), None) if !old.exists() && moved_by_hand(&old, &new) => {
                repo.git
                    .run(&["worktree", "repair", &new.to_string_lossy()])?;
                if !repo.worktrees()?.iter().any(|w| {
                    w.path == canonical(&new) && w.branch.as_deref() == Some(UPDATE_BRANCH)
                }) {
                    bail!(
                        "git did not record the update worktree at {} after `git worktree repair`",
                        new.display()
                    );
                }
                println!("the update worktree is already at {}", new.display());
            }
            _ if fs::symlink_metadata(&new).is_ok() => bail!(
                "{new} already exists, and the update worktree would be moved there; \
                 if it is the update worktree, moved there by hand, run `git worktree repair {new}`, \
                 then `git intake migrate` again; otherwise move it out of the way first",
                new = new.display()
            ),
            // No worktree is checked out, as while the update waits for its
            // merge request: only the place it is recorded at changes.
            (None, _) if !old.exists() => {
                println!(
                    "the update has no worktree checked out; it is recorded at {}",
                    new.display()
                );
            }
            _ => {
                let started_inside = repo.started_in(&old);
                repo.move_update_worktree(&old, &new)?;
                println!(
                    "moved the update worktree from {} to {}",
                    old.display(),
                    new.display()
                );
                println!(
                    "processes, shells and editors that used {} still point at the old path; \
                     stop them and open the update worktree at {}",
                    old.display(),
                    new.display()
                );
                if started_inside {
                    moved_from_here = Some(new.clone());
                }
            }
        }
        repo.exclude_update_worktree()?;
        new
    };

    // Checks run in the old location, or at a path containing `/.git` that
    // a bare repository keeps, may not have seen the tree as it is. Once
    // `finish` has begun, the verified commit is what it publishes, and the
    // verification stays its record.
    let why = if new != old {
        Some("which ran in the old location of the update worktree".to_string())
    } else if blind(&new) {
        Some(format!(
            "which ran at {}, a path containing `/.git` that compilers, bundlers and watchers ignore, \
             so the checks may not have seen the tree as it is",
            new.display()
        ))
    } else {
        None
    };
    let mut state = state.into_current(new);
    if let Some(why) = why
        && state.finishing.is_none()
        && state.merge_request.is_none()
        && let Some(v) = state.verification.take()
    {
        println!("dropped the verification of {}, {why}", abbrev(&v.commit));
    }
    repo.save_state(&state)?;
    println!(
        "migrated the update in progress to state format {}; next: {}",
        state::current_format(),
        next_step(&state, &Git::worktree(&state.worktree))?
    );
    if let Some(new) = moved_from_here {
        eprintln!(
            "note: the current directory was in the update worktree, which moved to {}",
            new.display()
        );
    }
    Ok(Exit::Success)
}

/// Whether the update worktree recorded at `wt` stays where it is: a bare
/// repository keeps it in its git directory, and one outside the git
/// directory is where it belongs already.
fn stays(repo: &Repo, wt: &Path) -> bool {
    *repo.main_worktree() == MainWorktree::Bare || !resolved(wt).starts_with(repo.common_dir())
}

/// `path` with its nearest existing ancestor resolved, for a path that may
/// no longer exist.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    for dir in path.ancestors() {
        if let Ok(dir) = fs::canonicalize(dir) {
            return rest
                .iter()
                .rev()
                .fold(dir, |acc: PathBuf, part| acc.join(part));
        }
        if let Some(name) = dir.file_name() {
            rest.push(name.to_os_string());
        }
    }
    path.to_path_buf()
}

/// Whether checks run in the update worktree at `wt` may miss files: tools
/// ignore every path containing `/.git`, the rule by which `start` refuses
/// such a placement.
fn blind(wt: &Path) -> bool {
    wt.to_string_lossy().contains("/.git")
}

/// Whether `new` is the worktree git still records at `old`: its `.git` file
/// names an administrative directory whose `gitdir` file names `old`.
fn moved_by_hand(old: &Path, new: &Path) -> bool {
    let Some(admin) = pointer(&new.join(".git"), "gitdir: ") else {
        return false;
    };
    pointer(&admin.join("gitdir"), "")
        .is_some_and(|recorded| recorded == resolved(&old.join(".git")))
}

/// The path a git link file at `file` names after `prefix`, resolved against
/// the file's directory when relative.
fn pointer(file: &Path, prefix: &str) -> Option<PathBuf> {
    let text = fs::read_to_string(file).ok()?;
    let target = Path::new(text.lines().next()?.strip_prefix(prefix)?.trim());
    Some(resolved(&file.parent()?.join(target)))
}
