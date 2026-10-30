//! `finish`: fast-forward and publish a verified update.
//!
//! `finish` is re-entrant. It validates the move of the integration branch
//! first, persists it in the update state, and only then moves it. Running it
//! again after an interruption or a rejected push completes the move (a
//! branch already at its planned value counts as done), publishes exactly
//! the planned commit, and cleans up. `--no-publish` closes the
//! update locally without pushing. In merge-request mode publication goes
//! through [`merge_request`](super::merge_request) instead.
//!
//! When the integration branch moved past the update's base, `finish` takes
//! the new tip in by a catch-up merge (see [`catch_up`](super::catch_up)),
//! verifies the result as `verify` does, and publishes it. It makes at most
//! one catch-up merge per run.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::Exit;
use crate::commands::update::require_worktree;
use crate::commands::{catch_up, merge_request, verify};
use crate::config::PublishVia;
use crate::git::{Git, abbrev, branch_ref, tracking_ref};
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
    let integration = state.config.integration.clone();
    // The remote pushed to, when this run publishes by push.
    let remote = state
        .config
        .publish
        .as_ref()
        .filter(|p| p.via == PublishVia::Push && !no_publish)
        .map(|p| p.remote.clone());
    let finishing = match state.finishing.clone() {
        Some(f) => {
            println!("resuming finish");
            match resume(repo, &mut state, f, no_publish, remote.as_deref())? {
                Ok(f) => f,
                Err(exit) => return Ok(exit),
            }
        }
        None => {
            require_worktree(repo, &state)?;
            let caught = match catch_up::conclude_for_finish(repo, &mut state)? {
                Ok(caught) => caught,
                Err(exit) => return Ok(exit),
            };
            require_finishable(&state)?;
            if let Some(remote) = &remote {
                follow_published(repo, remote, &integration)?;
                state.remote_tip = repo.git.commit(&tracking_ref(remote, &integration))?;
                repo.save_state(&state)?;
            }
            let tip = local_tip(&repo.git, &integration)?;
            let recheck = |repo: &Repo, state: &mut UpdateState| -> Result<Option<String>> {
                if let Some(remote) = &remote {
                    fetch(&repo.git, remote)?;
                    if let Some(published) = repo.git.commit(&tracking_ref(remote, &integration))?
                        && !repo.git.is_ancestor(&published, &state.base)?
                    {
                        return Ok(Some(format!(
                            "'{remote}/{integration}' moved to {}",
                            abbrev(&published)
                        )));
                    }
                }
                let tip = local_tip(&repo.git, &integration)?;
                Ok((tip != state.base)
                    .then(|| format!("'{integration}' moved to {}", abbrev(&tip))))
            };
            let run = Run {
                moved: false,
                caught,
                remote_tip: false,
            };
            let moves = match prepare(repo, &mut state, &tip, run, &recheck)? {
                Ready::Publish(_) => plan(repo, &state, &tip)?,
                Ready::Holds(head) => held(repo, &state, &tip, &head)?,
                Ready::Stop(exit) => return Ok(exit),
            };
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
            let unpushed = not_yet_published(repo, &publish, &finishing.moves)?;
            if unpushed.is_empty() {
                println!(
                    "'{publish}' already holds {}; nothing was pushed",
                    branches.join(", ")
                );
            } else if let Err(err) = push_exact(repo, &publish, &unpushed)? {
                if let Some(f) = state.finishing.as_mut() {
                    f.push_error = Some(err.clone());
                }
                repo.save_state(&state)?;
                bail!(
                    "the local integration branch is advanced, but pushing to '{publish}' was rejected: {err}\n\
                     Nothing was published, and the update stays in its finishing step. Run `git intake finish` \
                     again: it retries the same push, or, when '{publish}/{integration}' moved past the update's base, \
                     catches up with it, verifies the result and pushes that. `git intake finish --no-publish` \
                     closes the update locally. Never force-push."
                );
            } else {
                println!("pushed {} to '{publish}'", branches.join(", "));
            }
        }
        None => println!("no publish remote is configured; nothing was pushed"),
    }
    close(repo, state, unpublished)
}

/// `finish` run again in its finishing step. Without publication it closes
/// the update at the planned commit, dropping a catch-up in progress. In push
/// mode it fetches the publish remote and, when the remote's integration
/// branch moved past the base since the move, catches up with it from the
/// finishing step, verifies the result and plans its move instead. Returns
/// the moves to complete, or the exit status when it stops.
fn resume(
    repo: &Repo,
    state: &mut UpdateState,
    mut f: Finishing,
    no_publish: bool,
    remote: Option<&str>,
) -> Result<std::result::Result<Finishing, Exit>> {
    let integration = state.config.integration.clone();
    let Some(planned) = f.moves.iter().find(|m| m.branch == integration).cloned() else {
        return Ok(Ok(f));
    };
    let wt = Git::worktree(&state.worktree);
    if no_publish {
        return close_locally(repo, state, f, &planned).map(Ok);
    }
    let Some(remote) = remote else {
        return Ok(Ok(f));
    };
    let git = &repo.git;
    let caught = match catch_up::conclude_for_finish(repo, state)? {
        Ok(caught) => caught,
        Err(exit) => return Ok(Err(exit)),
    };
    let head = if state.worktree.exists() {
        wt.commit("HEAD")?
    } else {
        None
    };
    fetch(git, remote)?;
    let published = git.commit(&tracking_ref(remote, &integration))?;
    // A catch-up made from this step is still to be verified or planned.
    let caught_up = head.is_some_and(|h| h != planned.new) || state.catching_up().is_some();
    // A remote that no longer holds what the update last took from it
    // dropped work a push would publish again.
    let rewound = match (&state.remote_tip, &published) {
        (Some(seen), Some(p)) => !git.is_ancestor(seen, p)?,
        _ => false,
    };
    let published = match published {
        // The remote did not move past the planned commit, or received it:
        // retry the same push, or skip it.
        None => return Ok(Ok(f)),
        Some(p) if git.is_ancestor(&planned.new, &p)? => return Ok(Ok(f)),
        Some(p) if rewound => {
            bail!(
                "'{remote}/{integration}' was rewritten: it is at {}, which no longer contains {}, its tip when \
                 the update last took it, so pushing again would publish what it dropped. No branch moved. \
                 Run `git intake finish --no-publish` to close the update locally",
                abbrev(&p),
                abbrev(state.remote_tip.as_deref().unwrap_or_default())
            );
        }
        Some(p) if caught_up => p,
        Some(p) if git.is_ancestor(&p, &planned.new)? => return Ok(Ok(f)),
        Some(p) => p,
    };
    let local = git.commit(&branch_ref(&integration))?;
    if local.as_deref() != Some(planned.new.as_str()) {
        bail!(
            "'{remote}/{integration}' moved to {} after `finish` moved '{integration}' to the planned commit {}, \
             and '{integration}' is now at {}, so the update makes no catch-up merge. No branch moved. \
             Run `git intake finish --no-publish` to close the update locally, then reconcile '{integration}' with \
             '{remote}/{integration}' and publish the result through a new verified update or the repository's usual review",
            abbrev(&published),
            abbrev(&planned.new),
            local.as_deref().map(abbrev).unwrap_or("nothing")
        );
    }
    require_worktree(repo, state)?;
    let recheck = |repo: &Repo, state: &mut UpdateState| -> Result<Option<String>> {
        fetch(&repo.git, remote)?;
        let now = repo.git.commit(&tracking_ref(remote, &integration))?;
        Ok(match now {
            Some(n) if !repo.git.is_ancestor(&n, &state.base)? => {
                Some(format!("'{remote}/{integration}' moved to {}", abbrev(&n)))
            }
            _ => None,
        })
    };
    let run = Run {
        moved: true,
        caught,
        remote_tip: true,
    };
    let head = match prepare(repo, state, &published, run, &recheck)? {
        Ready::Publish(head) | Ready::Holds(head) => head,
        Ready::Stop(exit) => return Ok(Err(exit)),
    };
    check(repo, state, &head)?;
    // The integration branch moves on from the planned commit, which the
    // catch-up merge contains.
    let m = PlannedMove {
        branch: integration,
        old: planned.old.clone(),
        new: head,
    };
    validate_checkouts(repo, std::slice::from_ref(&m))?;
    f.moves = vec![m];
    f.push_error = None;
    state.finishing = Some(f.clone());
    repo.save_state(state)?;
    Ok(Ok(f))
}

/// `finish --no-publish` in the finishing step: closes the update at the
/// planned commit, or lands locally a verified catch-up made from the
/// finishing step instead. A catch-up merge still in conflict is dropped
/// after its resolutions are recorded, and a concluded one that is not
/// verified is dropped, naming the commits it drops.
fn close_locally(
    repo: &Repo,
    state: &mut UpdateState,
    mut f: Finishing,
    planned: &PlannedMove,
) -> Result<Finishing> {
    let wt = Git::worktree(&state.worktree);
    if state.catching_up().is_some() {
        if state.worktree.exists() && wt.merge_in_progress()? {
            catch_up::abandon(repo, state)?;
            println!(
                "the update closes at the planned commit {}",
                abbrev(&planned.new)
            );
            return Ok(f);
        }
        if catch_up::conclude_for_finish(repo, state)?.is_err() {
            // Merged again, it stopped on conflicts: dropped as well.
            catch_up::abandon(repo, state)?;
            return Ok(f);
        }
    }
    let head = if state.worktree.exists() {
        wt.commit("HEAD")?
    } else {
        None
    };
    let Some(head) = head.filter(|h| *h != planned.new) else {
        return Ok(f);
    };
    let verified = state
        .verification
        .as_ref()
        .is_some_and(|v| v.passed && v.commit == head);
    if !verified {
        // Dropped with the update branch: named, so that they can still be
        // found until git prunes unreachable commits.
        let dropped = crate::git::lines(&repo.git.run(&[
            "log",
            "--format=%h %s",
            &head,
            &format!("^{}", planned.new),
            "--",
        ])?);
        println!(
            "dropped the catch-up of the update, which is not verified; the update closes at the planned commit {}. \
             The dropped commits stay in the repository until git prunes unreachable commits:",
            abbrev(&planned.new)
        );
        for c in &dropped {
            println!("  {c}");
        }
        return Ok(f);
    }
    check(repo, state, &head)?;
    let m = PlannedMove {
        branch: planned.branch.clone(),
        old: planned.old.clone(),
        new: head.clone(),
    };
    validate_checkouts(repo, std::slice::from_ref(&m))?;
    println!(
        "lands the verified catch-up of the update, {}, instead of the planned commit {}",
        abbrev(&head),
        abbrev(&planned.new)
    );
    f.moves = vec![m];
    f.push_error = None;
    state.finishing = Some(f.clone());
    repo.save_state(state)?;
    Ok(f)
}

/// What this run of `finish` has done so far, for [`prepare`].
#[derive(Clone, Copy)]
pub struct Run {
    /// `finish` has moved the integration branch already.
    pub moved: bool,
    /// This run concluded a catch-up merge, which counts as its one
    /// catch-up.
    pub caught: bool,
    /// The integration tip is the publish remote's, which may lack commits
    /// of a local integration branch the base was taken from. A local tip
    /// is the base itself, so any other is a move.
    pub remote_tip: bool,
}

/// The update as `prepare` leaves it.
pub enum Ready {
    /// The verified update head, ready to land on the integration tip.
    Publish(String),
    /// The integration tip already contains the verified update head.
    Holds(String),
    /// `finish` stops with this exit status: on conflicts, failed checks, or
    /// an integration branch that moved again.
    Stop(Exit),
}

/// Readies the update to land on the integration tip `tip`. The update
/// head must be verified, or be a catch-up merge the tool made, which is
/// verified here. When `tip` contains the verified head, the update needs
/// nothing more. When `tip` is otherwise not the base, the update catches
/// up with it and the catch-up merge is verified. After a verification,
/// `recheck` tells whether the integration branch moved again meanwhile, in
/// which case `finish` stops: it makes at most one catch-up per run. A
/// remote tip the base contains, as a publish remote behind a local
/// integration branch that holds commits of its own, is not a move; a local
/// tip other than the base is, and one that does not contain the base was
/// rewritten.
pub fn prepare(
    repo: &Repo,
    state: &mut UpdateState,
    tip: &str,
    run: Run,
    recheck: &dyn Fn(&Repo, &mut UpdateState) -> Result<Option<String>>,
) -> Result<Ready> {
    let mut head = require_finishable(state)?;
    let wt = Git::worktree(&state.worktree);
    let verified = |state: &UpdateState, head: &str| {
        state
            .verification
            .as_ref()
            .is_some_and(|v| v.passed && v.commit == head)
    };
    let git = &repo.git;
    if verified(state, &head) && git.is_ancestor(&head, tip)? {
        println!(
            "'{}' at {} already contains the verified commit {}",
            state.config.integration,
            abbrev(tip),
            abbrev(&head)
        );
        return Ok(Ready::Holds(head));
    }
    let mut verify = !verified(state, &head);
    let moved_on = if run.remote_tip {
        !git.is_ancestor(tip, &state.base)?
    } else {
        tip != state.base
    };
    if moved_on && run.caught {
        // The catch-up this run concluded took in an earlier tip: nothing
        // is verified or moved before the next run catches up again.
        println!(
            "'{}' moved to {} since the catch-up merge was made; no branch moved to the update. \
             Run `git intake finish` again to catch up with it",
            state.config.integration,
            abbrev(tip)
        );
        return Ok(Ready::Stop(Exit::Negative));
    }
    if moved_on {
        catch_up::require_possible(repo, state, tip, run.moved, run.remote_tip)?;
        if run.remote_tip {
            state.remote_tip = Some(tip.to_string());
        }
        if let Some(exit) = catch_up::run(repo, state, tip)? {
            return Ok(Ready::Stop(exit));
        }
        head = wt.line(&["rev-parse", "HEAD"])?;
        verify = true;
    }
    if verify {
        if !verify::verify_head(repo, state, &head)? {
            println!(
                "verification of the catch-up merge {} failed; no branch moved to the update. Fix it on the update \
                 branch in {}, commit, and run `git intake verify`, or run `git intake finish` to verify it again",
                abbrev(&head),
                state.worktree.display()
            );
            return Ok(Ready::Stop(Exit::Negative));
        }
        println!("verified {}", abbrev(&head));
    }
    if verify && let Some(why) = recheck(repo, state)? {
        println!(
            "{why} while the update was verified; no branch moved to the update. \
             Run `git intake finish` again to catch up with it"
        );
        return Ok(Ready::Stop(Exit::Negative));
    }
    Ok(Ready::Publish(head))
}

/// Refuses, before anything moves, an update that `finish` cannot publish:
/// a queued upstream is not merged, or its plan not carried out; the update
/// worktree has uncommitted changes or a merge in progress; or its head is
/// neither the verified commit nor a catch-up merge the tool made. Returns
/// the update head.
pub fn require_finishable(state: &UpdateState) -> Result<String> {
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged, or their plans not carried out, yet: {}; run `git intake continue`",
            open.join(", ")
        );
    }
    let head = clean_head(state, &Git::worktree(&state.worktree))?;
    let verified = state
        .verification
        .as_ref()
        .is_some_and(|v| v.passed && v.commit == head);
    if !verified && !catch_up::unverified_head(state, &head) {
        require_verified(state, &head)?;
    }
    Ok(head)
}

/// The update head, refusing uncommitted changes or a merge in progress in
/// the update worktree.
fn clean_head(state: &UpdateState, wt: &Git) -> Result<String> {
    if wt.merge_in_progress()? || !wt.tracked_changes()?.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes or a merge in progress; commit or discard them and run `git intake verify` again",
            state.worktree.display()
        );
    }
    wt.line(&["rev-parse", "HEAD"])
}

/// Refuses an update head that is not the verified commit.
fn require_verified(state: &UpdateState, head: &str) -> Result<()> {
    match &state.verification {
        None => bail!("the update is not verified; run `git intake verify`"),
        Some(v) if v.commit != head => bail!(
            "the update branch is at {} but {} was verified; run `git intake verify` for the new commit",
            abbrev(head),
            abbrev(&v.commit)
        ),
        Some(v) if !v.passed => bail!(
            "verification of {} failed; fix the update and run `git intake verify` again",
            abbrev(&v.commit)
        ),
        Some(_) => Ok(()),
    }
}

/// The local integration branch's tip.
fn local_tip(git: &Git, integration: &str) -> Result<String> {
    git.commit(&branch_ref(integration))?
        .with_context(|| format!("the integration branch '{integration}' does not exist"))
}

/// Fetches the publish remote, refusing with git's reason when it fails.
fn fetch(git: &Git, remote: &str) -> Result<()> {
    if git.remote_url(remote)?.is_none() {
        bail!("the publish remote '{remote}' does not exist");
    }
    // Read where the remote is fetched from, as every other comparison with
    // its tracking refs is. If the push side is ahead, the push, never
    // forced, is rejected instead.
    let out = git.raw(&["fetch", "--quiet", "--no-tags", remote])?;
    if !out.success() {
        bail!(
            "cannot fetch the publish remote '{remote}': {}; no branch moved",
            out.into_error()
        );
    }
    Ok(())
}

/// Brings the integration branch up to date with the publish remote's:
/// fetches the remote and, when its integration branch holds commits the
/// local one lacks and the local one holds none it lacks, fast-forwards the
/// local one to it, as `start` and `finish` do. Refuses when the fetch fails
/// or the two have diverged. Returns whether it moved the branch.
pub fn follow_published(repo: &Repo, remote: &str, integration: &str) -> Result<bool> {
    let git = &repo.git;
    fetch(git, remote)?;
    let local = local_tip(git, integration)?;
    let Some(published) = git.commit(&tracking_ref(remote, integration))? else {
        return Ok(false);
    };
    let missing = git.count(&local, &published)?;
    if missing == 0 {
        return Ok(false);
    }
    let extra = git.count(&published, &local)?;
    if extra > 0 {
        bail!(
            "'{integration}' and '{remote}/{integration}' have diverged: '{integration}' lacks {missing} commit{} of \
             '{remote}/{integration}', which lacks {extra} of '{integration}'. No branch moved. The local commits must be \
             published or reconciled first: merge '{remote}/{integration}' into '{integration}' and publish the result, \
             or reset '{integration}' to it",
            plural(missing)
        );
    }
    let m = PlannedMove {
        branch: integration.to_string(),
        old: Some(local.clone()),
        new: published.clone(),
    };
    move_branch(repo, &m, &local, "git-intake: follow the publish remote")?;
    println!(
        "fast-forwarded '{integration}' from {} to {} to match '{remote}/{integration}'",
        abbrev(&local),
        abbrev(&published)
    );
    Ok(true)
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
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

/// The planned moves whose commit the publish remote's branch, as last
/// fetched, does not contain yet. A branch that already holds the planned
/// commit, as one moved past it by other means, needs no push, and a push of
/// it would not be a fast-forward.
fn not_yet_published(repo: &Repo, remote: &str, moves: &[PlannedMove]) -> Result<Vec<PlannedMove>> {
    let mut unpushed = Vec::new();
    for m in moves {
        let published = repo.git.commit(&tracking_ref(remote, &m.branch))?;
        let holds = match &published {
            Some(p) => repo.git.is_ancestor(&m.new, p)?,
            None => false,
        };
        if !holds {
            unpushed.push(m.clone());
        }
    }
    Ok(unpushed)
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
    state::save(&repo.paths.last_update, &state::in_current_format(&state).0)?;
    repo.remove_update_artifacts()?;
    repo.remove_plan()?;
    upstreams::release(&repo.git, &state)?;
    state::remove(&repo.paths.state)?;
    println!("update finished");
    Ok(Exit::Success)
}

/// Checks that the verified update head `head` can land on the base by
/// fast-forward: it contains the base, every merged target, and every copy
/// and journal commit of a port. Moves nothing.
pub fn check(repo: &Repo, state: &UpdateState, head: &str) -> Result<()> {
    let git = &repo.git;
    let integration = &state.config.integration;
    if !git.is_ancestor(&state.base, head)? {
        bail!(
            "the update branch at {} does not contain the base {}; '{integration}' can only fast-forward. \
             Run `git intake abort` and start again",
            abbrev(head),
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
            if !git.is_ancestor(c, head)? {
                missing.push(format!("{} at {}", q.name, abbrev(c)));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "the update branch at {} does not contain the targets, copies or journal commits of:\n{}\n\
             run `git intake abort` and start again",
            abbrev(head),
            bullet(&missing)
        );
    }
    Ok(())
}

/// Validates the update and plans the move of the integration branch from
/// its tip `tip`, which is the base, to the verified commit; moves nothing.
fn plan(repo: &Repo, state: &UpdateState, tip: &str) -> Result<Vec<PlannedMove>> {
    let wt = Git::worktree(&state.worktree);
    let head = clean_head(state, &wt)?;
    require_verified(state, &head)?;
    let integration = state.config.integration.clone();
    if tip != state.base {
        bail!(
            "'{integration}' is at {}, not at the update's base {}; run `git intake finish` again to catch up with it",
            abbrev(tip),
            abbrev(&state.base)
        );
    }
    check(repo, state, &head)?;
    let moves = vec![PlannedMove {
        branch: integration,
        old: Some(state.base.clone()),
        new: head,
    }];
    validate_checkouts(repo, &moves)?;
    Ok(moves)
}

/// The move of an integration branch at `tip`, which already contains the
/// verified commit `head`: nothing moves, and the planned commit is `head`.
fn held(repo: &Repo, state: &UpdateState, tip: &str, head: &str) -> Result<Vec<PlannedMove>> {
    check(repo, state, head)?;
    Ok(vec![PlannedMove {
        branch: state.config.integration.clone(),
        old: Some(tip.to_string()),
        new: head.to_string(),
    }])
}

/// Every checkout of a branch about to move must be able to follow it.
pub fn validate_checkouts(repo: &Repo, moves: &[PlannedMove]) -> Result<()> {
    for m in moves {
        if let Some(from) = move_from(&repo.git, m)? {
            for dir in repo.checkouts_of(&m.branch)? {
                validate_checkout(&dir, &m.branch, &from, &m.new)?;
            }
        }
    }
    Ok(())
}

/// Where a planned move goes from: the branch's value before `finish`, or a
/// commit along the way to the planned one, as the planned commit of an
/// earlier move that a catch-up merge contains. `None` when the branch has
/// nothing to move, being at the planned commit or past it, or does not
/// exist yet.
fn move_from(git: &Git, m: &PlannedMove) -> Result<Option<String>> {
    let Some(current) = git.commit(&branch_ref(&m.branch))? else {
        return Ok(None);
    };
    if git.is_ancestor(&m.new, &current)? {
        return Ok(None);
    }
    Ok(Some(current))
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

/// Performs one planned move, or confirms it is already done. A branch at
/// the planned commit or past it never moves; one along the way from its
/// value before `finish` to the planned commit moves on from there.
pub fn apply(repo: &Repo, m: &PlannedMove) -> Result<()> {
    let git = &repo.git;
    let current = git.commit(&branch_ref(&m.branch))?;
    if current.as_deref() == Some(m.new.as_str()) {
        println!("{}: at {}", m.branch, abbrev(&m.new));
        return Ok(());
    }
    if let Some(c) = &current
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
    let along = match (&current, &m.old) {
        (Some(c), Some(old)) if current != m.old => {
            git.is_ancestor(old, c)? && git.is_ancestor(c, &m.new)?
        }
        _ => false,
    };
    if current != m.old && !along {
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
    let from = current.clone().unwrap_or_default();
    move_branch(repo, m, &from, "git-intake: finish")?;
    println!(
        "{}: {} -> {}",
        m.branch,
        current.as_deref().map(abbrev).unwrap_or("(new)"),
        abbrev(&m.new)
    );
    Ok(())
}

/// Moves `m.branch` from `from`, an empty value for a branch that does not
/// exist, to `m.new`: a checkout of it by `git merge --ff-only`, refusing
/// when it would lose changes, and the branch otherwise by `update-ref`
/// against `from`.
fn move_branch(repo: &Repo, m: &PlannedMove, from: &str, reason: &str) -> Result<()> {
    match repo.checkouts_of(&m.branch)?.first() {
        Some(dir) if !from.is_empty() => {
            validate_checkout(dir, &m.branch, from, &m.new)?;
            Git::worktree(dir).run(&["merge", "--ff-only", "--quiet", &m.new])?;
        }
        _ => {
            repo.git.run(&[
                "update-ref",
                "-m",
                reason,
                &branch_ref(&m.branch),
                &m.new,
                from,
            ])?;
        }
    }
    Ok(())
}
