//! `finish`: publish a verified update.
//!
//! In push mode, `finish` pushes exactly the verified update head to the
//! publish remote's integration branch, without force, and moves no local
//! branch. When that branch moved past the update's base, `finish` takes the
//! new tip in by a catch-up merge (see [`catch_up`](super::catch_up)),
//! verifies the result as `verify` does, and publishes it. It makes at most
//! one catch-up merge per run. A rejected push publishes nothing and keeps
//! the update, so running `finish` again fetches the remote and pushes
//! again, or catches up first when the remote moved. In merge-request mode
//! the update is published as a merge request through
//! [`merge_request`](super::merge_request) instead.

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::merge_request::Announced;
use crate::commands::update::require_worktree;
use crate::commands::{catch_up, merge_request, verify};
use crate::config::{PublishVia, Upstream};
use crate::git::{Git, abbrev, branch_ref};
use crate::identity;
use crate::output::say;
use crate::repo::{Bootstrap, NamedBy, Repo, bullet};
use crate::state::{self, Stored, UpdateState, Work};
use crate::upstreams;

pub fn run(repo: &Repo) -> Result<Exit> {
    run_announced(repo, &mut Announced::default())
}

/// `finish` within a run that may have named the update's changes to its
/// own checks already, as `sync` does before it verifies.
pub fn run_announced(repo: &Repo, announced: &mut Announced) -> Result<Exit> {
    // An update an earlier release left waiting for its merge request is
    // the merge request's from now on.
    if let Some(Stored::Waiting(waiting)) = repo.stored()? {
        return merge_request::hand_over(repo, &waiting);
    }
    let state = repo.require_state()?;
    match state.config.publish.via {
        PublishVia::MergeRequest => merge_request::finish(repo, state),
        PublishVia::Push => by_push(repo, state, announced),
    }
}

/// Pushes the verified update to the publish remote's integration branch,
/// catching up first when that branch moved past the base.
fn by_push(repo: &Repo, mut state: UpdateState, announced: &mut Announced) -> Result<Exit> {
    let integration = state.config.integration.clone();
    let remote = state.config.publish.remote.clone();
    // Refused before anything is pushed, as the configuration was read
    // before the remote may have changed.
    identity::Upstreams::new(&repo.git, &state.config.upstreams)?
        .refuse_push(&repo.git, &remote)?;
    require_worktree(repo, &state)?;
    let caught = match catch_up::conclude_for_finish(repo, &mut state)? {
        Ok(caught) => caught,
        Err(exit) => return Ok(exit),
    };
    require_finishable(&state)?;
    let from = Bootstrap::of(&state.config, NamedBy::Update);
    let tip = repo.integration_tip(&from)?;
    let recheck = |repo: &Repo, state: &mut UpdateState| -> Result<Option<String>> {
        let now = repo.integration_tip(&from)?;
        Ok((now != state.base)
            .then(|| format!("'{remote}/{integration}' moved to {}", abbrev(&now))))
    };
    let head = match prepare(repo, &mut state, &tip, caught, &recheck, announced)? {
        Ready::Publish(head) => head,
        Ready::Holds(head) => {
            announced.check_changes(repo, &state, &head)?;
            check(repo, &state, &head)?;
            say!("nothing was pushed");
            return close(repo, state);
        }
        Ready::Stop(exit) => return Ok(exit),
    };
    announced.check_changes(repo, &state, &head)?;
    check(repo, &state, &head)?;
    if let Err(err) = push(repo, &remote, &integration, &head, &state.config.upstreams)? {
        bail!(
            "pushing {} to '{remote}/{integration}' was rejected: {err}\n\
             Nothing was published, and the update is kept, verified at {}. Run `git intake finish` again: it pushes \
             again, or, when '{remote}/{integration}' moved past the update's base, catches up with it, verifies \
             the result and pushes that. `git intake abort` ends the update. Never force-push.",
            abbrev(&head),
            abbrev(&head)
        );
    }
    say!("pushed {} to '{remote}/{integration}'", abbrev(&head));
    close(repo, state)
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
/// up with it and the catch-up merge is verified; a `tip` that does not
/// contain the base was rewritten and is refused. After a verification,
/// `recheck` tells whether the integration branch moved again meanwhile, in
/// which case `finish` stops: it makes at most one catch-up per run, and
/// `caught` says that this run concluded one already.
pub fn prepare(
    repo: &Repo,
    state: &mut UpdateState,
    tip: &str,
    caught: bool,
    recheck: &dyn Fn(&Repo, &mut UpdateState) -> Result<Option<String>>,
    announced: &mut Announced,
) -> Result<Ready> {
    let mut head = require_finishable(state)?;
    let wt = Git::worktree(&state.worktree);
    let git = &repo.git;
    let shown = format!(
        "'{}/{}'",
        state.config.publish.remote, state.config.integration
    );
    if state.verified_at(&head) && git.is_ancestor(&head, tip)? {
        say!(
            "{shown} at {} already contains the verified commit {}",
            abbrev(tip),
            abbrev(&head)
        );
        return Ok(Ready::Holds(head));
    }
    let mut verify = !state.verified_at(&head);
    let moved_on = tip != state.base;
    if moved_on && caught {
        // The catch-up this run concluded took in an earlier tip: nothing
        // is verified or pushed before the next run catches up again.
        say!(
            "{shown} moved to {} since the catch-up merge was made; nothing was pushed. \
             Run `git intake finish` again to catch up with it",
            abbrev(tip)
        );
        return Ok(Ready::Stop(Exit::Negative));
    }
    if moved_on {
        catch_up::require_possible(repo, state, tip, catch_up::Advice::Abort)?;
        if let Some(exit) = catch_up::run(repo, state, tip)? {
            return Ok(Ready::Stop(exit));
        }
        head = wt.line(&["rev-parse", "HEAD"])?;
        verify = true;
    }
    if verify {
        // Named before the checks run, since they run what the update
        // brings whether they pass or fail.
        announced.check_changes(repo, state, &head)?;
        if !verify::verify_head(repo, state, &head)? {
            say!(
                "verification of the catch-up merge {} failed; nothing was pushed. Fix it on the update branch in \
                 {}, commit, and run `git intake verify`, or run `git intake finish` to verify it again",
                abbrev(&head),
                state.worktree.display()
            );
            return Ok(Ready::Stop(Exit::Negative));
        }
        say!("verified {}", abbrev(&head));
    }
    if verify && let Some(why) = recheck(repo, state)? {
        say!(
            "{why} while the update was verified; nothing was pushed. \
             Run `git intake finish` again to catch up with it"
        );
        return Ok(Ready::Stop(Exit::Negative));
    }
    Ok(Ready::Publish(head))
}

/// Refuses, before anything is pushed, an update that `finish` cannot
/// publish: a queued upstream is not merged, or its plan not carried out;
/// the update worktree has uncommitted changes or a merge in progress; or
/// its head is neither the verified commit nor a catch-up merge the tool
/// made. Returns the update head.
pub fn require_finishable(state: &UpdateState) -> Result<String> {
    let open: Vec<&str> = state.unmerged().iter().map(|u| u.name.as_str()).collect();
    if !open.is_empty() {
        bail!(
            "upstreams are not merged, or their plans not carried out, yet: {}; run `git intake continue`",
            open.join(", ")
        );
    }
    let head = clean_head(state, &Git::worktree(&state.worktree))?;
    let verified = state.verified_at(&head);
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

/// Refuses an update head that is not the verified commit. A partial run
/// on the head does not count: without a full run there, the refusal names
/// the checks it left out, and a failed full run there is reported as such.
fn require_verified(state: &UpdateState, head: &str) -> Result<()> {
    let full_run_at_head = state
        .verification
        .as_ref()
        .is_some_and(|v| v.commit == head);
    if !full_run_at_head
        && let Some(p) = state.partial_run.as_ref().filter(|p| p.run.commit == head)
    {
        bail!(
            "the update head {} has only a partial run, which left out {} and does not verify the update; \
             run `git intake verify` without `--only` or `--skip` to run every check",
            abbrev(head),
            p.left_out
                .iter()
                .map(|n| format!("'{n}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
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

/// Pushes exactly the verified commit `head` to the integration branch of
/// the publish remote, without force: whatever any local branch points to
/// is never published by this update. Returns git's error when the remote
/// rejects the push.
fn push(
    repo: &Repo,
    remote: &str,
    integration: &str,
    head: &str,
    upstreams: &[Upstream],
) -> Result<std::result::Result<(), String>> {
    identity::Upstreams::new(&repo.git, upstreams)?.refuse_push(&repo.git, remote)?;
    let out = repo.git.raw(&[
        "push",
        "--quiet",
        remote,
        &format!("{head}:{}", branch_ref(integration)),
    ])?;
    Ok(if out.success() {
        Ok(())
    } else {
        Err(out.into_error().to_string())
    })
}

/// Completes an update: keeps it as the last update for `report` and
/// removes the worktree, the update branch, the plan, the update's upstream
/// references, its logs and the state.
pub fn close(repo: &Repo, mut state: UpdateState) -> Result<Exit> {
    state.finished = true;
    state::save(&repo.paths.last_update, &state)?;
    repo.remove_update_artifacts()?;
    repo.remove_plan()?;
    upstreams::release(&repo.git, &state)?;
    repo.remove_logs()?;
    repo.paths.remove(&repo.paths.state)?;
    say!("update finished");
    Ok(Exit::Success)
}

/// Checks that the update head `head` can land on the base by
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
