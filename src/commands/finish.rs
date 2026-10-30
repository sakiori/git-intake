//! `finish`: fast-forward and publish a verified update.
//!
//! `finish` is re-entrant. It validates the move of the integration branch
//! first, persists it in the update state, and only then moves it. Running it
//! again after an interruption or a rejected push completes the move (a
//! branch already at its planned value counts as done), publishes exactly
//! the planned commit, and cleans up. `--no-publish` closes the
//! update locally without pushing. In merge-request mode publication goes
//! through [`merge_request`](super::merge_request) instead.

use std::path::Path;

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::merge_request;
use crate::commands::update::require_worktree;
use crate::config::PublishVia;
use crate::git::{Git, abbrev, branch_ref};
use crate::repo::{Repo, bullet};
use crate::state::{self, Finishing, PlannedMove, UpdateState, Work};
use crate::upstreams;

pub fn run(repo: &Repo, no_publish: bool) -> Result<Exit> {
    let state = repo.require_state()?;
    if state.merge_request.is_some() {
        return merge_request::complete(repo, state, no_publish);
    }
    let via = state.config.publish.as_ref().map(|p| p.via);
    if via == Some(PublishVia::MergeRequest) && state.finishing.is_none() && !no_publish {
        return merge_request::open(repo, state);
    }
    finish_by_push(repo, state, no_publish)
}

/// Moves the integration branch locally and, in push mode, pushes it.
fn finish_by_push(repo: &Repo, mut state: UpdateState, no_publish: bool) -> Result<Exit> {
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
        // In merge-request mode this path is reached only by `--no-publish`
        // (or a resumed one): the integration branch is never pushed directly.
        Some(publish) if no_publish || publish.via == PublishVia::MergeRequest => {
            unpublished = branches.clone();
            println!(
                "not published to '{}' (advanced locally only): {}",
                publish.remote,
                branches.join(", ")
            );
        }
        Some(publish) => {
            let publish = publish.remote;
            if let Err(err) = push_exact(repo, &publish, &finishing.moves)? {
                if let Some(f) = state.finishing.as_mut() {
                    f.push_error = Some(err.clone());
                }
                repo.save_state(&state)?;
                bail!(
                    "the local integration branch is advanced, but pushing to '{publish}' was rejected: {err}\n\
                     Nothing was published, and the update stays in its finishing step. Run `git intake finish` \
                     to retry the same push, or `git intake finish --no-publish` to close the update locally. \
                     If '{publish}' holds work the local branch lacks, close the update with `finish --no-publish`, \
                     merge that work, and publish the result through a new verified update or the repository's usual review. \
                     Never force-push."
                );
            }
            println!("pushed {} to '{publish}'", branches.join(", "));
        }
        None => println!("no publish remote is configured; nothing was pushed"),
    }
    close(repo, state, unpublished)
}

/// Pushes exactly the planned commits without force: whatever the local
/// branches point to now is never published by this update. Returns git's
/// error when the remote rejects the push.
pub fn push_exact(
    repo: &Repo,
    remote: &str,
    moves: &[PlannedMove],
) -> Result<std::result::Result<(), String>> {
    let mut args = vec![
        "push".to_string(),
        "--quiet".to_string(),
        "--atomic".to_string(),
        remote.to_string(),
    ];
    for m in moves {
        args.push(format!("{}:{}", m.new, branch_ref(&m.branch)));
    }
    let out = repo.git.raw(&args)?;
    Ok(if out.success() {
        Ok(())
    } else {
        Err(out.into_error().to_string())
    })
}

/// Completes an update: keeps it as the last update for `report` and
/// removes the worktree, the update branch, the plan, the update's upstream
/// references and the state.
pub fn close(repo: &Repo, mut state: UpdateState, unpublished: Vec<String>) -> Result<Exit> {
    state.finished = true;
    if let Some(f) = state.finishing.as_mut() {
        f.push_error = None;
        f.unpublished = unpublished;
    }
    state::save(&repo.paths.last_update, &state)?;
    repo.remove_update_artifacts()?;
    repo.remove_plan()?;
    upstreams::release(&repo.git, &state)?;
    state::remove(&repo.paths.state)?;
    println!("update finished");
    Ok(Exit::Success)
}

/// Validates the update and plans the move of the integration branch;
/// moves nothing.
pub fn plan(repo: &Repo, state: &UpdateState) -> Result<Vec<PlannedMove>> {
    let git = &repo.git;
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged, or their plans not carried out, yet: {}; run `git intake continue`",
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
            "'{integration}' moved from {} to {} since the update started; the update must take the new integration tip and be verified again: \
             run `git intake abort`, then `git intake start` (recorded resolutions are reapplied), `verify` and `finish`",
            abbrev(&state.base),
            tip.as_deref().map(abbrev).unwrap_or("nothing")
        );
    }
    // The integration branch only fast-forwards: the verified commit must
    // contain the base, every merged target, and every copy and journal
    // commit of a port.
    if !git.is_ancestor(&state.base, &head)? {
        bail!(
            "the update branch at {} does not contain the base {}; '{integration}' can only fast-forward. \
             Run `git intake abort` and start again",
            abbrev(&head),
            abbrev(&state.base)
        );
    }
    let mut missing = Vec::new();
    for q in &state.upstreams {
        let commits: Vec<&String> = match &q.work {
            Work::Follow(f) => vec![&f.target],
            Work::Port(p) => p
                .commits
                .iter()
                .filter_map(|c| c.copied.as_ref())
                .chain(p.journal_commit.as_ref())
                .collect(),
        };
        for c in commits {
            if !git.is_ancestor(c, &head)? {
                missing.push(format!("{} at {}", q.name, abbrev(c)));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "the update branch at {} does not contain the targets, copies or journal commits of:\n{}\n\
             run `git intake abort` and start again",
            abbrev(&head),
            bullet(&missing)
        );
    }

    let moves = vec![PlannedMove {
        branch: integration.clone(),
        old: Some(state.base.clone()),
        new: head.clone(),
    }];
    validate_checkouts(repo, &moves)?;
    Ok(moves)
}

/// Every checkout of a branch about to move must be able to follow it.
pub fn validate_checkouts(repo: &Repo, moves: &[PlannedMove]) -> Result<()> {
    for m in moves {
        if let Some(old) = &m.old {
            for dir in repo.checkouts_of(&m.branch)? {
                validate_checkout(&dir, &m.branch, old, &m.new)?;
            }
        }
    }
    Ok(())
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
pub fn apply(repo: &Repo, m: &PlannedMove) -> Result<()> {
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
