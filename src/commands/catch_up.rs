//! Catching an update up with a moved integration branch.
//!
//! A catch-up merge takes the new integration tip into the update: a merge
//! whose first parent is the tip and whose second parent is the update
//! branch head, marked with the `Intake-Catch-Up` trailer. With the tip as
//! the first parent, the first-parent line of the integration branch stays
//! the line of commits the branch itself received, and the update arrives on
//! the second-parent side, as when a merge request is merged with a merge
//! commit. Everything already resolved and fixed on the update branch stays
//! as it is; only conflicts between the new integration commits and the
//! update need a hand.
//!
//! `git merge` run on the update branch would make the update head the first
//! parent, so the tool records the tip in the state first, checks it out
//! detached in the update worktree, merges the update head into it, and
//! moves the update branch from the head it recorded to the merge with
//! `update-ref`, then attaches the worktree to the branch again. The recorded
//! tip makes every point of interruption recognisable: a merge in progress
//! whose `MERGE_HEAD` is the recorded head, or a detached commit that carries
//! the trailer and whose parents are the recorded tip and head.

use anyhow::{Result, bail};

use crate::Exit;
use crate::commands::update::{
    label_conflicts, print_conflicted_files, rerere_resolved, save_merge_output, settle,
};
use crate::config::{self, Config};
use crate::git::{Git, abbrev, branch_ref};
use crate::port;
use crate::repo::{Repo, bullet};
use crate::state::{self, CatchUp, CatchUpStep, UPDATE_BRANCH, UpdateState};
use crate::sync_record;

/// Where a catch-up merge's output is kept, beside the upstreams' merges'.
fn output_path(repo: &Repo) -> std::path::PathBuf {
    repo.paths.logs.join("catch-up.out")
}

/// Refuses, naming what to do instead, when the update cannot take `tip` in
/// by a catch-up merge: `tip` does not contain the base, or the commits it
/// adds change what the update means. `moved` says that `finish` has moved
/// the integration branch, so that the update can only be closed at the
/// planned commit.
pub fn require_possible(repo: &Repo, state: &UpdateState, tip: &str, moved: bool) -> Result<()> {
    let git = &repo.git;
    let integration = &state.config.integration;
    let advice = if moved {
        "Run `git intake finish --no-publish` to close the update at the planned commit without publishing it"
    } else {
        "Run `git intake abort`, then `git intake start` to plan the update from the new history"
    };
    if !git.is_ancestor(&state.base, tip)? {
        bail!(
            "'{integration}' was rewritten: its tip {} does not contain the update's base {}, so the update \
             cannot catch up with it. Nothing was merged and no branch moved. {advice}",
            abbrev(tip),
            abbrev(&state.base)
        );
    }
    let mut problems = Vec::new();
    let queued: Vec<&str> = state.upstreams.iter().map(|q| q.name.as_str()).collect();
    let fetched: Vec<String> = state.upstreams.iter().map(|q| q.fetched.clone()).collect();
    for (commit, names) in sync_record::naming_upstreams(git, tip, &state.base, &fetched)? {
        for name in names.iter().filter(|n| queued.contains(&n.as_str())) {
            problems.push(format!(
                "{} carries a sync record or a copy of upstream '{name}', which the update takes in",
                abbrev(&commit)
            ));
        }
    }
    for q in state.upstreams.iter().filter(|q| q.port().is_some()) {
        let journal = port::journal_path(&q.name);
        let changed = git.run(&["diff", "--name-only", &state.base, tip, "--", &journal])?;
        if !changed.trim().is_empty() {
            problems.push(format!(
                "the journal {journal} of port upstream '{}' changed",
                q.name
            ));
        }
    }
    problems.extend(configuration_changes(git, &state.config, tip)?);
    if !problems.is_empty() {
        bail!(
            "cannot catch up with '{integration}' at {}: the commits it adds change what the update means:\n{}\n\
             Nothing was merged and no branch moved. {advice}",
            abbrev(tip),
            bullet(&problems)
        );
    }
    Ok(())
}

/// How the configuration at `tip` differs from the update's in anything but
/// its install command and checks, which the integration branch may change.
fn configuration_changes(git: &Git, ours: &Config, tip: &str) -> Result<Vec<String>> {
    let file = config::FILE_NAME;
    let Some(text) = git.show_file(tip, file)? else {
        return Ok(vec![format!("{file} was removed")]);
    };
    let theirs = match config::parse(&text) {
        Ok(theirs) => theirs,
        Err(err) => return Ok(vec![format!("{file} is invalid: {err:#}")]),
    };
    let mut changes = Vec::new();
    if theirs.integration != ours.integration {
        changes.push(format!(
            "{file} names '{}' as the integration branch instead of '{}'",
            theirs.integration, ours.integration
        ));
    }
    if theirs.publish != ours.publish {
        changes.push(format!("the publication setting in {file} changed"));
    }
    for up in &ours.upstreams {
        match theirs.upstream(&up.name) {
            None => changes.push(format!("upstream '{}' was removed from {file}", up.name)),
            Some(t) if t != up => changes.push(format!("upstream '{}' changed in {file}", up.name)),
            Some(_) => {}
        }
    }
    for up in &theirs.upstreams {
        if ours.upstream(&up.name).is_none() {
            changes.push(format!("upstream '{}' was added to {file}", up.name));
        }
    }
    Ok(changes)
}

/// Catches the update up with the integration tip `tip`, which must have
/// passed [`require_possible`]. Returns the exit status when the merge
/// stopped on conflicts; otherwise the update branch is at the catch-up
/// merge and the base is `tip`.
pub fn run(repo: &Repo, state: &mut UpdateState, tip: &str) -> Result<Option<Exit>> {
    let wt = Git::worktree(&state.worktree);
    if wt.merge_in_progress()? || !wt.tracked_changes()?.is_empty() {
        bail!(
            "the update worktree {} has uncommitted changes or a merge in progress; commit or discard them, \
             then run `git intake finish` again",
            state.worktree.display()
        );
    }
    let head_before = wt.line(&["rev-parse", "HEAD"])?;
    if repo.git.commit(&branch_ref(UPDATE_BRANCH))?.as_deref() != Some(head_before.as_str()) {
        bail!(
            "the update worktree {} is not at the head of '{UPDATE_BRANCH}'; check the branch out there, \
             then run `git intake finish` again",
            state.worktree.display()
        );
    }
    if repo.git.is_ancestor(&head_before, tip)? {
        bail!(
            "'{}' at {} already contains the update head {}, which was never verified; \
             run `git intake abort` to end the update",
            state.config.integration,
            abbrev(tip),
            abbrev(&head_before)
        );
    }
    state.catch_ups.push(CatchUp {
        tip: tip.to_string(),
        head_before,
        step: CatchUpStep::Merging,
        merge_commit: None,
        conflicts: Vec::new(),
    });
    repo.save_state(state)?;
    merge(repo, state)
}

/// Builds the recorded catch-up merge in the update worktree: checks out
/// the tip detached and merges the head the catch-up started from into it.
fn merge(repo: &Repo, state: &mut UpdateState) -> Result<Option<Exit>> {
    let wt = Git::worktree(&state.worktree);
    let c = current(state)?.clone();
    let integration = state.config.integration.clone();
    println!(
        "catching up with '{integration}' at {}: merging the update at {} into it",
        abbrev(&c.tip),
        abbrev(&c.head_before)
    );
    wt.run(&["checkout", "--quiet", "--detach", &c.tip])?;
    let msg = sync_record::catch_up_message(&integration, &repo.git.short(&c.tip)?);
    let output_path = output_path(repo);
    state::remove(&output_path)?;
    let out = wt.raw(&[
        "merge",
        "--no-ff",
        "--no-log",
        "--no-edit",
        "-m",
        &msg,
        &c.head_before,
    ])?;
    save_merge_output(&output_path, &out)?;
    if out.success() {
        let merge = wt.line(&["rev-parse", "HEAD"])?;
        conclude_at(repo, state, &merge)?;
        return Ok(None);
    }
    if !wt.merge_in_progress()? {
        // Refused before it began: the update stays as it was.
        wt.run(&["checkout", "--quiet", UPDATE_BRANCH])?;
        state.catch_ups.pop();
        repo.save_state(state)?;
        return Err(anyhow::Error::new(out.into_error()).context(format!(
            "catching up with '{integration}' at {} failed; nothing moved",
            abbrev(&c.tip)
        )));
    }
    let output = format!("{}{}", out.stdout, out.stderr);
    stop_on_conflict(repo, state, &output).map(Some)
}

fn current(state: &UpdateState) -> Result<&CatchUp> {
    match state.catching_up() {
        Some(c) => Ok(c),
        None => bail!("no catch-up merge is in progress"),
    }
}

fn current_mut(state: &mut UpdateState) -> Result<&mut CatchUp> {
    match state.catch_ups.last_mut() {
        Some(c) if c.step != CatchUpStep::Merged => Ok(c),
        _ => bail!("no catch-up merge is in progress"),
    }
}

/// Records the conflicts of the stopped catch-up merge and reports them.
fn stop_on_conflict(repo: &Repo, state: &mut UpdateState, output: &str) -> Result<Exit> {
    let wt = Git::worktree(&state.worktree);
    let conflicts = label_conflicts(&wt.unmerged_files()?, &rerere_resolved(output));
    let c = current_mut(state)?;
    c.conflicts = conflicts;
    c.step = CatchUpStep::Conflict;
    let c = c.clone();
    repo.save_state(state)?;
    let integration = &state.config.integration;
    println!(
        "conflicts catching up with '{integration}' at {} in {}: ours is '{integration}' at {}, \
         theirs is the update at {}",
        abbrev(&c.tip),
        state.worktree.display(),
        abbrev(&c.tip),
        abbrev(&c.head_before)
    );
    if c.conflicts.is_empty() {
        println!(
            "the merge stopped without conflicted files:\n{}",
            output.trim()
        );
    }
    print_conflicted_files(&c.conflicts);
    println!(
        "resolve the conflicts in {}, stage them with `git add`, then run `git intake continue`",
        state.worktree.display()
    );
    Ok(Exit::Negative)
}

/// Completes the catch-up merge in progress, if any. With `resolved`, as
/// `continue` runs it, a merge whose conflicts are all resolved and staged
/// is committed; otherwise a merge in progress is refused. A merge already
/// committed is never made again: the update branch is moved to it. A
/// catch-up that never got as far as its merge is merged again. Returns the
/// exit status when the merge stopped on conflicts.
pub fn conclude(repo: &Repo, state: &mut UpdateState, resolved: bool) -> Result<Option<Exit>> {
    let Some(c) = state.catching_up().cloned() else {
        return Ok(None);
    };
    let wt = Git::worktree(&state.worktree);
    let integration = state.config.integration.clone();
    if wt.merge_in_progress()? {
        if wt.commit("MERGE_HEAD")?.as_deref() != Some(c.head_before.as_str()) {
            bail!(
                "the merge in progress in {} is not the catch-up merge with '{integration}'; conclude or abort it by hand, \
                 or run `git intake abort`",
                state.worktree.display()
            );
        }
        if !resolved {
            bail!(
                "the catch-up merge with '{integration}' at {} is in progress in {}; resolve its conflicts, \
                 stage them with `git add`, then run `git intake continue`",
                abbrev(&c.tip),
                state.worktree.display()
            );
        }
        let unmerged = wt.unmerged_files()?;
        if !unmerged.is_empty() {
            bail!(
                "the catch-up merge with '{integration}' still has unresolved conflicts in {}:\n{}\n\
                 resolve them, stage them with `git add`, then run `git intake continue`",
                state.worktree.display(),
                bullet(&unmerged)
            );
        }
        let msg = sync_record::catch_up_message(&integration, &repo.git.short(&c.tip)?);
        wt.run(&["commit", "--quiet", "--no-edit", "-m", &msg])?;
    }
    let head = wt.line(&["rev-parse", "HEAD"])?;
    let branch = repo.git.commit(&branch_ref(UPDATE_BRANCH))?;
    if is_catch_up_merge(repo, &state.config.integration, &c, &head)? {
        conclude_at(repo, state, &head)?;
        return Ok(None);
    }
    // Moved before the state could say so.
    if let Some(b) = &branch
        && is_catch_up_merge(repo, &state.config.integration, &c, b)?
    {
        conclude_at(repo, state, b)?;
        return Ok(None);
    }
    // The merge never began, or was aborted by hand.
    if (head == c.tip || head == c.head_before) && branch.as_deref() == Some(c.head_before.as_str())
    {
        return merge(repo, state);
    }
    bail!(
        "the catch-up merge with '{integration}' at {} is no longer in progress in {}, and no commit there merges \
         {} into it; run `git intake abort` to end the update",
        abbrev(&c.tip),
        state.worktree.display(),
        abbrev(&c.head_before)
    )
}

/// Whether `commit` is the catch-up merge `c` describes: its parents are the
/// recorded tip and head, in that order, and it carries the trailer.
fn is_catch_up_merge(repo: &Repo, integration: &str, c: &CatchUp, commit: &str) -> Result<bool> {
    let line = repo
        .git
        .line(&["rev-list", "--parents", "-n", "1", commit, "--"])?;
    let parents: Vec<&str> = line.split_whitespace().skip(1).collect();
    Ok(parents == [c.tip.as_str(), c.head_before.as_str()]
        && sync_record::catch_up_of(&repo.git, commit)?.as_deref() == Some(integration))
}

/// Moves the update branch to the catch-up merge `merge` while it is still
/// at the head the catch-up started from, attaches the update worktree to
/// it, and records the merge: the base becomes the tip caught up with, the
/// update is unverified, and its install command and checks are read from
/// the configuration at the merge.
fn conclude_at(repo: &Repo, state: &mut UpdateState, merge: &str) -> Result<()> {
    let c = current(state)?.clone();
    let integration = state.config.integration.clone();
    let git = &repo.git;
    let full = branch_ref(UPDATE_BRANCH);
    let branch = git.commit(&full)?;
    if branch.as_deref() != Some(merge) {
        let out = git.raw(&[
            "update-ref",
            "-m",
            "git-intake: catch up",
            &full,
            merge,
            &c.head_before,
        ])?;
        if !out.success() {
            let now = git.commit(&full)?;
            bail!(
                "'{UPDATE_BRANCH}' is at {}, no longer at {} where the catch-up with '{integration}' at {} started, \
                 so it was not moved to the catch-up merge {}. No branch moved. Move '{UPDATE_BRANCH}' back to {} \
                 and run `git intake continue`, or run `git intake abort`",
                now.as_deref().map(abbrev).unwrap_or("nothing"),
                abbrev(&c.head_before),
                abbrev(&c.tip),
                abbrev(merge),
                abbrev(&c.head_before)
            );
        }
    }
    let wt = Git::worktree(&state.worktree);
    let attached = wt.raw(&["symbolic-ref", "--quiet", "HEAD"])?;
    if attached.stdout.trim() != full {
        wt.run(&["checkout", "--quiet", UPDATE_BRANCH])?;
    }
    if let Some(Ok(merged)) = git
        .show_file(merge, config::FILE_NAME)?
        .map(|text| config::parse(&text))
    {
        state.config.install = merged.install;
        state.config.checks = merged.checks;
    }
    let done = current_mut(state)?;
    settle(&mut done.conflicts);
    done.merge_commit = Some(merge.to_string());
    done.step = CatchUpStep::Merged;
    state.base = c.tip.clone();
    state.verification = None;
    repo.save_state(state)?;
    println!(
        "caught up with '{integration}' at {} as {}",
        abbrev(&c.tip),
        abbrev(merge)
    );
    Ok(())
}

/// Whether `head` is the update's last catch-up merge, with nothing
/// committed on top, and not verified yet.
pub fn unverified_head(state: &UpdateState, head: &str) -> bool {
    state
        .catch_ups
        .last()
        .is_some_and(|c| c.step == CatchUpStep::Merged && c.merge_commit.as_deref() == Some(head))
        && !state
            .verification
            .as_ref()
            .is_some_and(|v| v.passed && v.commit == head)
}
