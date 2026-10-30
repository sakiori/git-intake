//! `finish`: fast-forward and publish a verified update.

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::update::require_worktree;
use crate::git::{Git, abbrev, branch_ref};
use crate::repo::{Repo, bullet};
use crate::state::{self, UpstreamStep};

/// A planned fast-forward of a local branch.
struct Move {
    branch: String,
    old: Option<String>,
    new: String,
}

pub fn run(repo: &Repo) -> Result<Exit> {
    let mut state = repo.require_state()?;
    require_worktree(repo, &state)?;
    let git = &repo.git;
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged yet: {}; run `git fork-sync continue`",
            open.join(", ")
        );
    }
    let verification = match &state.verification {
        None => bail!("the update is not verified; run `git fork-sync verify`"),
        Some(v) if !v.passed => bail!(
            "verification of {} failed; fix the update and run `git fork-sync verify` again",
            abbrev(&v.commit)
        ),
        Some(v) => v.clone(),
    };
    let wt = Git::worktree(&state.worktree);
    if wt.merge_in_progress()? || !wt.tracked_changes()?.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes or a merge in progress; commit or discard them and run `git fork-sync verify` again",
            state.worktree.display()
        );
    }
    let head = wt.line(&["rev-parse", "HEAD"])?;
    if head != verification.commit {
        bail!(
            "the update branch is at {} but {} was verified; run `git fork-sync verify` for the new commit",
            abbrev(&head),
            abbrev(&verification.commit)
        );
    }
    let integration = state.config.integration.clone();
    let tip = git.commit(&branch_ref(&integration))?;
    if tip.as_deref() != Some(state.base.as_str()) {
        bail!(
            "'{integration}' moved from {} to {} since the update started; the update must merge the new integration tip and be verified again: \
             run `git fork-sync abort`, then `git fork-sync start` (recorded resolutions are reapplied), `verify` and `finish`",
            abbrev(&state.base),
            tip.as_deref().map(abbrev).unwrap_or("nothing")
        );
    }

    let mut moves = vec![Move {
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
        debug_assert_eq!(q.step, UpstreamStep::Merged);
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
        moves.push(Move {
            branch: mirror,
            old,
            new: q.target.clone(),
        });
    }
    for m in &moves {
        for dir in repo.checkouts_of(&m.branch)? {
            let changes = Git::worktree(&dir).tracked_changes()?;
            if !changes.is_empty() {
                bail!(
                    "'{}' is checked out in {} with uncommitted changes; commit or stash them first:\n{}",
                    m.branch,
                    dir.display(),
                    bullet(&changes)
                );
            }
        }
    }

    for m in &moves {
        fast_forward(repo, m)?;
        println!(
            "{}: {} -> {}",
            m.branch,
            m.old.as_deref().map(abbrev).unwrap_or("(new)"),
            abbrev(&m.new)
        );
    }
    state.finished = true;
    state::save(&repo.paths.last_update, &state)?;

    let push = match &state.config.publish {
        Some(publish) => {
            // All or nothing: either every branch is published or none moves.
            let mut args = vec![
                "push".to_string(),
                "--quiet".to_string(),
                "--atomic".to_string(),
                publish.clone(),
            ];
            for m in &moves {
                let r = branch_ref(&m.branch);
                args.push(format!("{r}:{r}"));
            }
            Some((publish.clone(), git.raw(&args)?))
        }
        None => None,
    };

    repo.remove_update_artifacts()?;
    state::remove(&repo.paths.state)?;

    match push {
        None => {
            println!("no publish remote is configured; nothing was pushed");
            println!("update finished");
            Ok(Exit::Success)
        }
        Some((publish, out)) if out.success() => {
            let names: Vec<&str> = moves.iter().map(|m| m.branch.as_str()).collect();
            println!("pushed {} to '{publish}'", names.join(", "));
            println!("update finished");
            Ok(Exit::Success)
        }
        Some((publish, out)) => bail!(
            "the local branches are advanced, but pushing to '{publish}' was rejected: {}\n\
             Someone else published first. Reconcile without force: run `git fetch {publish}`, merge \
             '{publish}/{integration}' into '{integration}' (or start a new update), verify the result, and push again.",
            out.into_error()
        ),
    }
}

/// Fast-forwards one branch, through its checkout when it is checked out.
fn fast_forward(repo: &Repo, m: &Move) -> Result<()> {
    let checkouts = repo.checkouts_of(&m.branch)?;
    if let Some(dir) = checkouts.first() {
        Git::worktree(dir).run(&["merge", "--ff-only", "--quiet", &m.new])?;
    } else {
        repo.git.run(&[
            "update-ref",
            "-m",
            "fork-sync: finish",
            &branch_ref(&m.branch),
            &m.new,
            m.old.as_deref().unwrap_or(""),
        ])?;
    }
    Ok(())
}
