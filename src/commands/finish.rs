//! `finish`: fast-forward and publish a verified update.
//!
//! `finish` is re-entrant. It validates every ref move first, persists the
//! planned moves in the update state, and only then moves refs. Running it
//! again after an interruption or a rejected push completes the remaining
//! moves (a branch already at its planned value counts as done), publishes
//! exactly the planned commits, and cleans up. `--no-publish` closes the
//! update locally without pushing.

use std::path::Path;

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::update::require_worktree;
use crate::git::{Git, abbrev, branch_ref};
use crate::repo::{Repo, bullet};
use crate::state::{self, Finishing, PlannedMove, UpdateState};

pub fn run(repo: &Repo, no_publish: bool) -> Result<Exit> {
    let mut state = repo.require_state()?;
    let finishing = match &state.finishing {
        Some(f) => {
            println!("resuming finish");
            f.clone()
        }
        None => {
            require_worktree(repo, &state)?;
            let moves = plan(repo, &state)?;
            let f = Finishing {
                moves,
                push_error: None,
                unpublished: Vec::new(),
            };
            state.finishing = Some(f.clone());
            repo.save_state(&state)?;
            f
        }
    };

    for m in &finishing.moves {
        apply(repo, m)?;
    }

    let branches: Vec<String> = finishing.moves.iter().map(|m| m.branch.clone()).collect();
    let mut unpublished = Vec::new();
    match state.config.publish.clone() {
        Some(publish) if no_publish => {
            unpublished = branches.clone();
            println!(
                "not published to '{publish}' (advanced locally only): {}",
                branches.join(", ")
            );
        }
        Some(publish) => {
            // All or nothing, and exactly the planned commits: whatever the
            // local branches point to now is never published by this update.
            let mut args = vec![
                "push".to_string(),
                "--quiet".to_string(),
                "--atomic".to_string(),
                publish.clone(),
            ];
            for m in &finishing.moves {
                args.push(format!("{}:{}", m.new, branch_ref(&m.branch)));
            }
            let out = repo.git.raw(&args)?;
            if !out.success() {
                let err = out.into_error().to_string();
                if let Some(f) = state.finishing.as_mut() {
                    f.push_error = Some(err.clone());
                }
                repo.save_state(&state)?;
                bail!(
                    "the local branches are advanced, but pushing to '{publish}' was rejected: {err}\n\
                     Nothing was published, and the update stays in its finishing step. Run `git intake finish` \
                     to retry the same push, or `git intake finish --no-publish` to close the update locally. \
                     If '{publish}' holds work the local branches lack, close the update with `finish --no-publish`, \
                     merge that work, and publish the result through a new verified update or the repository's usual review. \
                     Never force-push."
                );
            }
            println!("pushed {} to '{publish}'", branches.join(", "));
        }
        None => println!("no publish remote is configured; nothing was pushed"),
    }

    state.finished = true;
    if let Some(f) = state.finishing.as_mut() {
        f.push_error = None;
        f.unpublished = unpublished;
    }
    state::save(&repo.paths.last_update, &state)?;
    repo.remove_update_artifacts()?;
    state::remove(&repo.paths.state)?;
    println!("update finished");
    Ok(Exit::Success)
}

/// Validates the update and plans every branch move; moves nothing.
fn plan(repo: &Repo, state: &UpdateState) -> Result<Vec<PlannedMove>> {
    let git = &repo.git;
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged yet: {}; run `git intake continue`",
            open.join(", ")
        );
    }
    let verification = match &state.verification {
        None => bail!("the update is not verified; run `git intake verify`"),
        Some(v) if !v.passed => bail!(
            "verification of {} failed; fix the update and run `git intake verify` again",
            abbrev(&v.commit)
        ),
        Some(v) => v.clone(),
    };
    let wt = Git::worktree(&state.worktree);
    if wt.merge_in_progress()? || !wt.tracked_changes()?.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes or a merge in progress; commit or discard them and run `git intake verify` again",
            state.worktree.display()
        );
    }
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if head != verification.commit {
        bail!(
            "the update branch is at {} but {} was verified; run `git intake verify` for the new commit",
            abbrev(&head),
            abbrev(&verification.commit)
        );
    }
    let integration = state.config.integration.clone();
    let tip = git.commit(&branch_ref(&integration))?;
    if tip.as_deref() != Some(state.base.as_str()) {
        bail!(
            "'{integration}' moved from {} to {} since the update started; the update must merge the new integration tip and be verified again: \
             run `git intake abort`, then `git intake start` (recorded resolutions are reapplied), `verify` and `finish`",
            abbrev(&state.base),
            tip.as_deref().map(abbrev).unwrap_or("nothing")
        );
    }
    // Branches only fast-forward: the verified commit must contain the base
    // and every merged target.
    if !git.is_ancestor(&state.base, &head)? {
        bail!(
            "the update branch at {} does not contain the base {}; '{integration}' can only fast-forward. \
             Run `git intake abort` and start again",
            abbrev(&head),
            abbrev(&state.base)
        );
    }
    let missing: Vec<String> = state
        .upstreams
        .iter()
        .filter(|q| !git.is_ancestor(&q.target, &head).unwrap_or(false))
        .map(|q| format!("{} at {}", q.name, abbrev(&q.target)))
        .collect();
    if !missing.is_empty() {
        bail!(
            "the update branch at {} does not contain the targets of:\n{}\nrun `git intake abort` and start again",
            abbrev(&head),
            bullet(&missing)
        );
    }

    let mut moves = vec![PlannedMove {
        branch: integration.clone(),
        old: Some(state.base.clone()),
        new: head.clone(),
    }];
    for q in &state.upstreams {
        let Some(mirror) = state
            .config
            .upstream(&q.name)
            .and_then(|u| u.mirror.clone())
        else {
            continue;
        };
        let old = git.commit(&branch_ref(&mirror))?;
        if let Some(old) = &old {
            if *old == q.target {
                continue;
            }
            if !git.is_ancestor(old, &q.target)? {
                bail!(
                    "mirror branch '{mirror}' at {} is not contained in upstream '{}' target {}; it cannot be fast-forwarded",
                    abbrev(old),
                    q.name,
                    abbrev(&q.target)
                );
            }
        }
        moves.push(PlannedMove {
            branch: mirror,
            old,
            new: q.target.clone(),
        });
    }
    for m in &moves {
        if let Some(old) = &m.old {
            for dir in repo.checkouts_of(&m.branch)? {
                validate_checkout(&dir, &m.branch, old, &m.new)?;
            }
        }
    }
    Ok(moves)
}

/// A checkout of a branch that is about to move must be able to follow the
/// fast-forward: no uncommitted changes and no untracked file in the way.
fn validate_checkout(dir: &Path, branch: &str, old: &str, new: &str) -> Result<()> {
    let git = Git::worktree(dir);
    let changes = git.tracked_changes()?;
    if !changes.is_empty() {
        bail!(
            "'{branch}' is checked out in {} with uncommitted changes; commit or stash them first:\n{}",
            dir.display(),
            bullet(&changes)
        );
    }
    let out = git.raw(&["read-tree", "-n", "-m", "-u", old, new])?;
    if !out.success() {
        bail!(
            "'{branch}' is checked out in {} and cannot follow the fast-forward: {}\nmove those files away, then run `git intake finish` again",
            dir.display(),
            out.stderr.trim()
        );
    }
    Ok(())
}

/// Performs one planned move, or confirms it is already done.
fn apply(repo: &Repo, m: &PlannedMove) -> Result<()> {
    let git = &repo.git;
    let current = git.commit(&branch_ref(&m.branch))?;
    if current.as_deref() == Some(m.new.as_str()) {
        println!("{}: at {}", m.branch, abbrev(&m.new));
        return Ok(());
    }
    if let Some(c) = &current
        && current != m.old
        && git.is_ancestor(&m.new, c)?
    {
        // Moved on after this update's move, for example by a reconciling merge.
        println!(
            "{}: at {}, which contains {}",
            m.branch,
            abbrev(c),
            abbrev(&m.new)
        );
        return Ok(());
    }
    if current != m.old {
        bail!(
            "'{}' is at {}, neither its value before finish ({}) nor the planned {}; it was moved outside the update. \
             Restore it or move it to {} by hand, then run `git intake finish` again",
            m.branch,
            current.as_deref().map(abbrev).unwrap_or("nothing"),
            m.old.as_deref().map(abbrev).unwrap_or("nothing"),
            abbrev(&m.new),
            abbrev(&m.new)
        );
    }
    match (repo.checkouts_of(&m.branch)?.first(), &m.old) {
        (Some(dir), Some(old)) => {
            validate_checkout(dir, &m.branch, old, &m.new)?;
            Git::worktree(dir).run(&["merge", "--ff-only", "--quiet", &m.new])?;
        }
        _ => {
            git.run(&[
                "update-ref",
                "-m",
                "git-intake: finish",
                &branch_ref(&m.branch),
                &m.new,
                m.old.as_deref().unwrap_or(""),
            ])?;
        }
    }
    println!(
        "{}: {} -> {}",
        m.branch,
        m.old.as_deref().map(abbrev).unwrap_or("(new)"),
        abbrev(&m.new)
    );
    Ok(())
}
